//! Most-claims yearbook board.

use benjisponge::data::podrick_models::PANTS_PARTICIPANTS;
use jiff::ToSpan;

use super::{
    BoardCard, BoardEntry, YearWindow,
    util::{competition_rank, plural},
};

#[derive(Clone, Copy, Debug, Default)]
struct ClaimsRow {
    participant_index: usize,
    claims: usize,
    claim_days: usize,
}

pub(super) fn board(window: &YearWindow<'_>) -> BoardCard {
    let mut rows = std::array::from_fn(|participant_index| ClaimsRow {
        participant_index,
        ..ClaimsRow::default()
    });
    if let Some((start, end)) = window.scored_range() {
        let mut date = start;
        loop {
            for (participant_index, row) in rows.iter_mut().enumerate() {
                let claims = window
                    .by_date
                    .get(&date)
                    .map_or(0, |day| day.participants[participant_index].claims());
                row.claims += usize::from(claims);
                if claims > 0 {
                    row.claim_days += 1;
                }
            }
            if date == end {
                break;
            }
            date += 1.days();
        }
    }

    BoardCard {
        heading_id: format!("pants-board-{}-claims", window.selected_year),
        title: "Most claims",
        entries: ranked(rows),
    }
}

fn ranked(rows: [ClaimsRow; 3]) -> Vec<BoardEntry> {
    let mut ordered = rows;
    ordered.sort_by_key(|row| (std::cmp::Reverse(row.claims), row.participant_index));
    let mut previous_value = None;
    let mut rank = 0;
    ordered
        .into_iter()
        .enumerate()
        .map(|(position, row)| {
            let tied = row.claims > 0 && rows.iter().filter(|r| r.claims == row.claims).count() > 1;
            BoardEntry {
                rank: competition_rank(position, row.claims, tied, &mut previous_value, &mut rank),
                display_name: PANTS_PARTICIPANTS[row.participant_index].display_name,
                value: format!("{} {}", row.claims, plural(row.claims, "claim", "claims")),
                detail: format!(
                    "{} claim {}",
                    row.claim_days,
                    plural(row.claim_days, "day", "days")
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn competition_ranks_and_does_not_rank_zeroes() {
        let entries = ranked([
            ClaimsRow {
                participant_index: 0,
                claims: 2,
                claim_days: 2,
            },
            ClaimsRow {
                participant_index: 1,
                claims: 2,
                claim_days: 1,
            },
            ClaimsRow {
                participant_index: 2,
                claims: 1,
                claim_days: 1,
            },
        ]);
        assert_eq!(entries[0].rank, "T1");
        assert_eq!(entries[1].rank, "T1");
        assert_eq!(entries[2].rank, "3");
        assert_eq!(entries[0].detail, "2 claim days");
    }
}
