//! Most-doubles yearbook board.

use benjisponge::data::podrick_models::PANTS_PARTICIPANTS;
use jiff::ToSpan;

use super::{
    BoardCard, BoardEntry, YearWindow,
    util::{competition_rank, plural},
};

#[derive(Clone, Copy, Debug, Default)]
struct DoublesRow {
    participant_index: usize,
    doubles: usize,
}

pub(super) fn board(window: &YearWindow<'_>) -> BoardCard {
    let mut rows = std::array::from_fn(|participant_index| DoublesRow {
        participant_index,
        ..DoublesRow::default()
    });
    if let Some((start, end)) = window.scored_range() {
        let mut date = start;
        loop {
            for (participant_index, row) in rows.iter_mut().enumerate() {
                let claims = window
                    .by_date
                    .get(&date)
                    .map_or(0, |day| day.participants[participant_index].claims());
                row.doubles += usize::from(claims == 2);
            }
            if date == end {
                break;
            }
            date += 1.days();
        }
    }

    BoardCard {
        heading_id: format!("pants-board-{}-doubles", window.selected_year),
        title: "Most doubles",
        entries: ranked(rows),
    }
}

fn ranked(rows: [DoublesRow; 3]) -> Vec<BoardEntry> {
    let mut ordered = rows;
    ordered.sort_by_key(|row| (std::cmp::Reverse(row.doubles), row.participant_index));
    let mut previous_value = None;
    let mut rank = 0;
    ordered
        .into_iter()
        .enumerate()
        .map(|(position, row)| {
            let tied =
                row.doubles > 0 && rows.iter().filter(|r| r.doubles == row.doubles).count() > 1;
            BoardEntry {
                rank: competition_rank(position, row.doubles, tied, &mut previous_value, &mut rank),
                display_name: PANTS_PARTICIPANTS[row.participant_index].display_name,
                value: format!(
                    "{} {}",
                    row.doubles,
                    plural(row.doubles, "double", "doubles")
                ),
                detail: format!(
                    "{} {} on double days",
                    row.doubles * 2,
                    plural(row.doubles * 2, "claim", "claims")
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroes_stay_unranked() {
        let entries = ranked([
            DoublesRow {
                participant_index: 0,
                doubles: 0,
            },
            DoublesRow {
                participant_index: 1,
                doubles: 0,
            },
            DoublesRow {
                participant_index: 2,
                doubles: 0,
            },
        ]);
        assert!(entries.iter().all(|entry| entry.rank == "—"));
        assert_eq!(entries[0].detail, "0 claims on double days");
    }
}
