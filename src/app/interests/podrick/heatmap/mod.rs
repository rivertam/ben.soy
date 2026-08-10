//! Calendar-year Pants Off heatmaps and yearbook boards.
//!
//! Source messages remain the stored facts. This module applies the shared
//! Eastern-time rules, pads each selected year to complete Sunday-to-Saturday
//! weeks, and renders one calendar per participant plus co-located yearbook
//! boards for claims, streaks, doubles, and crew totals.

mod claims;
mod crew;
mod doubles;
mod person;
mod streak;
mod util;

use std::collections::BTreeMap;

use benjisponge::data::podrick_models::{
    PANTS_PARTICIPANTS, PantsDay, PodrickPantsMessage, aggregate_pants_messages,
    classify_pants_message, classify_pants_time,
};
use jiff::{ToSpan, civil::Date};
use topcoat::{
    Result,
    view::{class, component, view},
};

use super::status::PantsStatus;
use crew::CrewTotals;
use person::{PersonCalendar, person_heatmap};
use util::{DAYS_PER_WEEK, META, NOTE, format_short, inclusive_dates, plural};

const YEAR_LINK: &str = "inline-flex min-h-[2rem] min-w-[3.2rem] items-center justify-center \
     rounded-[0.15rem] border border-hairline px-2 text-ink2 hover:border-oxide hover:text-oxide \
     focus-visible:border-oxide focus-visible:text-oxide focus-visible:outline-solid \
     focus-visible:outline-2 focus-visible:outline-oxide focus-visible:outline-offset-2";
const YEAR_CURRENT: &str = "inline-flex min-h-[2rem] min-w-[3.2rem] items-center justify-center \
     rounded-[0.15rem] border border-ink bg-ink px-2 text-card";
pub(super) const HEAT_FILL: &str =
    "bg-[color-mix(in_srgb,var(--color-oxide)_var(--pants-heat-alpha,0%),var(--color-card))]";
const LEGEND_CELL: &str =
    "relative inline-block size-[0.72rem] rounded-[0.12rem] border border-hairline/88";
pub(super) const DOT: &str =
    "absolute right-[0.05rem] top-[0.05rem] size-[0.18rem] rounded-full bg-patina";
/// Two thick bars crossed into an X; redder than the claim heat fill.
pub(super) const INFARCTION_A: &str = "absolute left-[10%] top-1/2 h-[0.14rem] w-[80%] -translate-y-1/2 \
     -rotate-45 bg-[color-mix(in_srgb,var(--color-oxide-hot)_55%,#8a0f08)]";
pub(super) const INFARCTION_B: &str = "absolute left-[10%] top-1/2 h-[0.14rem] w-[80%] -translate-y-1/2 \
     rotate-45 bg-[color-mix(in_srgb,var(--color-oxide-hot)_55%,#8a0f08)]";
pub(super) const WORM: &str =
    "absolute inset-0 flex items-center justify-center text-[0.42rem] leading-none select-none";
pub(super) const CELL_ASYNC: &str =
    "ring-1 ring-[color-mix(in_srgb,var(--color-patina)_38%,transparent)]";
const BOARD: &str = "rounded-[0.2rem] border border-hairline bg-card/35 p-4";

#[component]
pub(super) async fn pants_heatmaps(
    status: PantsStatus,
    now: i64,
    selected_year: i16,
    earliest_year: i16,
    current_year: i16,
) -> Result {
    let state = if !status.database_available {
        Some("The Pants Off store is unavailable right now.")
    } else if !status.history_seeded {
        Some(
            "Podrick is still walking the channel's history. The calendars appear when that one-time seed finishes.",
        )
    } else if status.messages.is_empty() {
        Some("History is synced, but none of the three participants has posted yet.")
    } else {
        None
    };

    let year = if state.is_none() {
        PantsYear::from_messages(&status.messages, now, selected_year)
    } else {
        None
    };
    let years: Vec<i16> = (earliest_year..=current_year).collect();

    view! {
        <section aria-labelledby="pants-heatmap-title">
            <header class="flex flex-wrap items-end justify-between gap-x-8 gap-y-4">
                <div>
                    <p class=(META)>"pants off"</p>
                    <h2
                        id="pants-heatmap-title"
                        class="mt-1 font-display text-2xl font-semibold tracking-tight"
                    >
                        (format!("{selected_year} claims"))
                    </h2>
                    <nav
                        class="mt-3 flex flex-wrap items-center gap-[0.35rem] \
                             font-meta text-[0.7rem] leading-none"
                        aria-label="Pants Off years"
                    >
                        for year in years.iter() {
                            if *year == selected_year {
                                <span class=(YEAR_CURRENT) aria-current="page">
                                    (year.to_string())
                                </span>
                            } else {
                                <a
                                    class=(YEAR_LINK)
                                    href=(year_href(*year, current_year))
                                    data-pants-year-link=""
                                >
                                    (year.to_string())
                                </a>
                            }
                        }
                    </nav>
                </div>
                <div
                    class="flex max-w-[30rem] flex-wrap items-center gap-x-3 gap-y-1 \
                         font-meta text-[0.61rem] leading-none uppercase text-muted"
                    aria-label="Legend"
                >
                    <span class="inline-flex items-center gap-1">
                        <span
                            class=(class!(LEGEND_CELL, HEAT_FILL))
                            style="--pants-heat-alpha: 34%"
                            aria-hidden="true"
                        ></span>
                        "one claim"
                    </span>
                    <span class="inline-flex items-center gap-1">
                        <span
                            class=(class!(LEGEND_CELL, HEAT_FILL))
                            style="--pants-heat-alpha: 82%"
                            aria-hidden="true"
                        ></span>
                        "double"
                    </span>
                    <span class="inline-flex items-center gap-1">
                        <span class=(class!(LEGEND_CELL, "ring-1 ring-patina")) aria-hidden="true">
                            <span class=(WORM)>"🪱"</span>
                        </span>
                        "kwerm"
                    </span>
                    <span class="inline-flex items-center gap-1">
                        <span class=(class!(LEGEND_CELL, CELL_ASYNC)) aria-hidden="true">
                            <span class=(class!(WORM, "opacity-45"))>"🪱"</span>
                        </span>
                        "asynkwerm"
                    </span>
                    <span class="inline-flex items-center gap-1">
                        <span
                            class=(class!(LEGEND_CELL, HEAT_FILL))
                            style="--pants-heat-alpha: 34%"
                            aria-hidden="true"
                        >
                            <span class=(DOT)></span>
                        </span>
                        "out of town"
                    </span>
                    <span class="inline-flex items-center gap-1">
                        <span class=(LEGEND_CELL) aria-hidden="true">
                            <span class=(INFARCTION_A)></span>
                            <span class=(INFARCTION_B)></span>
                        </span>
                        "infarction"
                    </span>
                </div>
            </header>

            if let Some(message) = state {
                <p class=(class!(NOTE, "mt-5 max-w-prose"))>(message)</p>
            } else if let Some(year) = year {
                <p class=(class!(NOTE, "mt-4"))>
                    (year.range_label.as_str())
                </p>
                <div class="mt-6 space-y-7">
                    for person in year.people.iter() {
                        person_heatmap(
                            person: person,
                            column_style: year.column_style.as_str(),
                            chart_style: year.chart_style.as_str(),
                            month_labels: year.month_labels.as_slice()
                        )
                    }
                </div>

                <section class="mt-12" aria-labelledby="pants-leaderboards-title">
                    <header class="max-w-prose">
                        <p class=(META)>"yearbook"</p>
                        <h3
                            id="pants-leaderboards-title"
                            class="mt-1 font-display text-xl font-semibold"
                        >
                            (format!("{selected_year} leaderboards"))
                        </h3>
                        <p class=(class!(NOTE, "mt-1"))>
                            "Each board resets January 1. A streak is consecutive Eastern dates "
                            "with at least one claim."
                        </p>
                    </header>
                    <div class="mt-5 grid gap-4 xl:grid-cols-3">
                        board_card(board: &year.claims)
                        board_card(board: &year.streak)
                        board_card(board: &year.doubles)
                    </div>
                    <section
                        class=(class!(BOARD, "mt-4"))
                        aria-labelledby="pants-crew-totals-title"
                    >
                        <div class="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-2">
                            <dl class="flex flex-wrap gap-x-8 gap-y-3">
                                <div>
                                    <dt class=(META)>"kwerms"</dt>
                                    <dd class="mt-1 font-display text-xl tabular-nums">
                                        (year.crew.kwerms)
                                    </dd>
                                    <dd class=(NOTE)>
                                        (format!(
                                            "{} AM · {} PM · {} {}",
                                            year.crew.am_kwerms,
                                            year.crew.pm_kwerms,
                                            year.crew.kwerm_days,
                                            plural(year.crew.kwerm_days, "day", "days")
                                        ))
                                    </dd>
                                </div>
                                <div>
                                    <dt class=(META)>"asynkwerms"</dt>
                                    <dd class="mt-1 font-display text-xl tabular-nums">
                                        (year.crew.asynkwerms)
                                    </dd>
                                </div>
                            </dl>
                        </div>
                    </section>
                </section>
            } else {
                <p class=(class!(NOTE, "mt-5 max-w-prose"))>
                    "The stored timestamps could not be projected into a calendar."
                </p>
            }
        </section>
    }
}

#[component]
async fn board_card(board: &BoardCard) -> Result {
    view! {
        <section class=(BOARD) aria-labelledby=(board.heading_id.as_str())>
            <h4
                id=(board.heading_id.as_str())
                class="font-display text-base font-semibold"
            >
                (board.title)
            </h4>
            <table class="mt-3 w-full table-fixed border-collapse text-left">
                <thead class="sr-only">
                    <tr>
                        <th>"Rank"</th>
                        <th>"Participant"</th>
                        <th>"Result"</th>
                    </tr>
                </thead>
                <tbody>
                    for entry in board.entries.iter() {
                        <tr class="border-t border-hairline/75 align-top">
                            <td
                                class="w-9 py-2 pr-2 font-meta text-[0.65rem] \
                                     tabular-nums text-muted"
                            >
                                (entry.rank.as_str())
                            </td>
                            <th
                                class="py-2 pr-3 font-body text-[0.76rem] \
                                     font-normal leading-snug text-ink2"
                                scope="row"
                            >
                                (entry.display_name)
                            </th>
                            <td class="w-[6.4rem] py-2 text-right">
                                <strong
                                    class="block font-meta text-[0.72rem] \
                                         font-normal tabular-nums text-ink"
                                >
                                    (entry.value.as_str())
                                </strong>
                                <span
                                    class="mt-0.5 block font-meta text-[0.58rem] \
                                         leading-snug text-muted"
                                >
                                    (entry.detail.as_str())
                                </span>
                            </td>
                        </tr>
                    }
                </tbody>
            </table>
        </section>
    }
}

pub(super) fn pants_year_bounds(status: &PantsStatus, now: i64) -> Option<(i16, i16)> {
    let current_year = classify_pants_time(now)?.date.year();
    let earliest_year = if status.database_available && status.history_seeded {
        status
            .messages
            .iter()
            .filter_map(classify_pants_message)
            .map(|message| message.date.year())
            .min()
            .unwrap_or(current_year)
    } else {
        current_year
    };
    Some((earliest_year.min(current_year), current_year))
}

pub(super) fn year_path(year: i16, current_year: i16) -> String {
    if year == current_year {
        "/podrick".to_string()
    } else {
        format!("/podrick?year={year}")
    }
}

fn year_href(year: i16, current_year: i16) -> String {
    format!("{}#pants-heatmap-title", year_path(year, current_year))
}

/// Shared yearbook card chrome. Each board module builds one of these from its
/// own query rather than sharing a metric enum.
pub(super) struct BoardCard {
    pub heading_id: String,
    pub title: &'static str,
    pub entries: Vec<BoardEntry>,
}

pub(super) struct BoardEntry {
    pub rank: String,
    pub display_name: &'static str,
    pub value: String,
    pub detail: String,
}

struct PantsYear {
    range_label: String,
    column_style: String,
    chart_style: String,
    month_labels: Vec<MonthLabel>,
    people: Vec<PersonCalendar>,
    claims: BoardCard,
    streak: BoardCard,
    doubles: BoardCard,
    crew: CrewTotals,
}

impl PantsYear {
    fn from_messages(
        messages: &[PodrickPantsMessage],
        now: i64,
        selected_year: i16,
    ) -> Option<Self> {
        let today = classify_pants_time(now)?.date;
        let days = aggregate_pants_messages(messages);
        let tracking_start = messages
            .iter()
            .filter_map(classify_pants_message)
            .map(|message| message.date)
            .min()?;
        Self::new(days, tracking_start, today, selected_year)
    }

    fn new(
        days: Vec<PantsDay>,
        tracking_start: Date,
        today: Date,
        selected_year: i16,
    ) -> Option<Self> {
        let year_start = Date::new(selected_year, 1, 1).ok()?;
        let year_end = Date::new(selected_year, 12, 31).ok()?;
        let start_offset = i64::from(year_start.weekday().to_sunday_zero_offset());
        let end_offset =
            DAYS_PER_WEEK as i64 - 1 - i64::from(year_end.weekday().to_sunday_zero_offset());
        let grid_start = year_start.checked_add((-start_offset).days()).ok()?;
        let grid_end = year_end.checked_add(end_offset.days()).ok()?;
        let dates = inclusive_dates(grid_start, grid_end)?;
        let week_count = dates.len() / DAYS_PER_WEEK;
        let month_labels = MonthLabel::for_year(selected_year, &dates);
        let by_date: BTreeMap<Date, PantsDay> =
            days.into_iter().map(|day| (day.date, day)).collect();
        let window = YearWindow {
            by_date: &by_date,
            year_start,
            year_end,
            tracking_start,
            today,
            selected_year,
        };
        let people = PANTS_PARTICIPANTS
            .iter()
            .enumerate()
            .map(|(participant_index, participant)| {
                PersonCalendar::build(participant, participant_index, &dates, &window)
            })
            .collect();

        Some(Self {
            range_label: format!(
                "{} – {} · {week_count} complete calendar weeks",
                format_short(year_start),
                format_short(year_end)
            ),
            column_style: format!("grid-template-columns: repeat({week_count}, minmax(0, 1fr))"),
            chart_style: format!(
                "grid-template-columns: repeat({week_count}, minmax(0, 1fr)); \
                 aspect-ratio: {week_count} / 7"
            ),
            month_labels,
            people,
            claims: claims::board(&window),
            streak: streak::board(&window),
            doubles: doubles::board(&window),
            crew: CrewTotals::query(&window),
        })
    }
}

/// The selected calendar year clipped to tracking and "today", plus the day map
/// each board queries independently.
pub(super) struct YearWindow<'a> {
    pub by_date: &'a BTreeMap<Date, PantsDay>,
    pub year_start: Date,
    pub year_end: Date,
    pub tracking_start: Date,
    pub today: Date,
    pub selected_year: i16,
}

impl YearWindow<'_> {
    pub fn scored_range(&self) -> Option<(Date, Date)> {
        let start = self.year_start.max(self.tracking_start);
        let end = self.year_end.min(self.today);
        (start <= end).then_some((start, end))
    }
}

pub(super) struct MonthLabel {
    pub label: String,
    pub column: usize,
    pub style: String,
}

impl MonthLabel {
    fn for_year(year: i16, dates: &[Date]) -> Vec<Self> {
        (1..=12)
            .filter_map(|month| {
                let first = Date::new(year, month, 1).ok()?;
                let column = dates.iter().position(|date| *date == first)? / DAYS_PER_WEEK + 1;
                Some(Self {
                    label: first.strftime("%b").to_string(),
                    column,
                    style: format!("grid-column: {column}"),
                })
            })
            .collect()
    }
}

#[cfg(test)]
pub(super) mod fixtures {
    use super::*;
    use benjisponge::data::podrick_models::{PANTS_CHANNEL_ID, aggregate_pants_messages};
    use jiff::Timestamp;

    pub(super) fn epoch(instant: &str) -> i64 {
        instant.parse::<Timestamp>().unwrap().as_second()
    }

    pub(super) fn message(id: &str, participant: usize, instant: &str) -> PodrickPantsMessage {
        PodrickPantsMessage {
            id: id.to_string(),
            message_id: id.to_string(),
            channel_id: PANTS_CHANNEL_ID.to_string(),
            author_id: PANTS_PARTICIPANTS[participant].author_id.to_string(),
            posted_at: epoch(instant),
        }
    }

    pub(super) struct OwnedWindow {
        by_date: BTreeMap<Date, PantsDay>,
        year_start: Date,
        year_end: Date,
        tracking_start: Date,
        today: Date,
        selected_year: i16,
    }

    impl OwnedWindow {
        pub(super) fn window(&self) -> YearWindow<'_> {
            YearWindow {
                by_date: &self.by_date,
                year_start: self.year_start,
                year_end: self.year_end,
                tracking_start: self.tracking_start,
                today: self.today,
                selected_year: self.selected_year,
            }
        }
    }

    pub(super) fn window_from(
        messages: &[PodrickPantsMessage],
        year_start: Date,
        year_end: Date,
        tracking_start: Date,
        today: Date,
        selected_year: i16,
    ) -> OwnedWindow {
        let days = aggregate_pants_messages(messages);
        OwnedWindow {
            by_date: days.into_iter().map(|day| (day.date, day)).collect(),
            year_start,
            year_end,
            tracking_start,
            today,
            selected_year,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fixtures::{epoch, message};

    fn year(messages: &[PodrickPantsMessage], selected: i16) -> PantsYear {
        PantsYear::from_messages(messages, epoch("2026-07-28T16:00:00Z"), selected).unwrap()
    }

    #[test]
    fn ordinary_year_is_53_complete_weeks_with_january_through_december_labels() {
        let year = year(&[message("101", 0, "2025-01-01T11:07:00Z")], 2025);
        assert_eq!(year.people.len(), 3);
        assert_eq!(year.people[0].cells.len(), 371);
        assert_eq!(year.people[0].cells[0].date.to_string(), "2024-12-29");
        assert_eq!(
            year.people[0].cells.last().unwrap().date.to_string(),
            "2026-01-03"
        );
        assert_eq!(year.month_labels.len(), 12);
        assert_eq!(year.month_labels[0].label, "Jan");
        assert_eq!(year.month_labels[0].column, 1);
        assert_eq!(year.month_labels[11].label, "Dec");
        assert!(year.chart_style.contains("repeat(53,"));
    }

    #[test]
    fn leap_year_beginning_saturday_uses_54_weeks() {
        let tracking_start = Date::new(2028, 1, 1).unwrap();
        let year = PantsYear::new(Vec::new(), tracking_start, tracking_start, 2028).unwrap();
        assert_eq!(year.people[0].cells.len(), 378);
        assert_eq!(year.people[0].cells[0].date.to_string(), "2027-12-26");
        assert_eq!(
            year.people[0].cells.last().unwrap().date.to_string(),
            "2029-01-06"
        );
        assert!(year.chart_style.contains("repeat(54,"));
    }

    #[test]
    fn claims_and_all_overlays_remain_independent() {
        let messages = [
            message("101", 0, "2026-07-28T10:07:00Z"),
            message("102", 0, "2026-07-28T22:07:00Z"),
            message("103", 0, "2026-07-28T16:07:00Z"),
            message("104", 0, "2026-07-28T22:08:00Z"),
            message("201", 1, "2026-07-28T22:07:01Z"),
            message("301", 2, "2026-07-28T22:07:02Z"),
        ];
        let year = year(&messages, 2026);
        let cell = year.people[0]
            .cells
            .iter()
            .find(|cell| cell.date.to_string() == "2026-07-28")
            .unwrap();
        assert_eq!(cell.claims, 2);
        assert!(cell.out_of_town);
        assert!(cell.infarction);
        assert!(cell.kwerm);
        assert!(!cell.asynkwerm);
        assert!(cell.label.contains("2 claims — double"));
        assert!(cell.label.contains("out of town at 12:07 PM"));
        assert!(cell.label.contains("infarction at 6:08 PM"));
        assert!(cell.label.contains("kwerm (PM)"));
    }

    #[test]
    fn padding_never_leaks_adjacent_year_claims_or_overlays() {
        let messages = [
            message("001", 0, "2024-12-29T11:07:00Z"),
            message("002", 1, "2024-12-29T11:07:01Z"),
            message("003", 2, "2024-12-29T11:07:02Z"),
            message("101", 0, "2025-01-01T11:07:00Z"),
        ];
        let year = year(&messages, 2025);
        let padding = &year.people[0].cells[0];
        assert_eq!(padding.date.to_string(), "2024-12-29");
        assert_eq!(padding.claims, 0);
        assert!(!padding.kwerm);
        assert!(padding.label.contains("outside 2025"));
        assert_eq!(year.claims.entries[0].value, "1 claim");
    }

    #[test]
    fn year_bounds_use_eastern_dates_and_stored_history() {
        let status = PantsStatus {
            database_available: true,
            history_seeded: true,
            messages: vec![message("101", 0, "2024-12-31T11:07:00Z")],
        };
        assert_eq!(
            pants_year_bounds(&status, epoch("2026-01-01T04:30:00Z")),
            Some((2024, 2025))
        );
    }
}
