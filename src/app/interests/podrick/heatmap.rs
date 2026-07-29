//! Three no-JavaScript, calendar-year Pants Off heatmaps.
//!
//! Source messages remain the stored facts. This module applies the shared
//! Eastern-time rules, pads each selected year to complete Sunday-to-Saturday
//! weeks, and derives that year's claims, doubles, streaks, and crew totals.

use std::collections::BTreeMap;

use benjisponge::data::podrick_models::{
    PANTS_PARTICIPANTS, PantsDay, PantsParticipantDay, PantsSlot, PodrickPantsMessage,
    aggregate_pants_messages, classify_pants_message, classify_pants_time,
};
use jiff::{ToSpan, civil::Date};
use topcoat::{
    Result,
    view::{class, component, view},
};

use super::status::PantsStatus;

const DAYS_PER_WEEK: usize = 7;

const NOTE: &str = "font-meta text-[0.7rem] leading-[1.55] text-muted";
const META: &str =
    "font-meta text-[0.6875rem] leading-normal tracking-[0.13em] uppercase text-muted";
const YEAR_LINK: &str = "inline-flex min-h-[2rem] min-w-[3.2rem] items-center justify-center \
     rounded-[0.15rem] border border-hairline px-2 text-ink2 hover:border-oxide hover:text-oxide \
     focus-visible:border-oxide focus-visible:text-oxide focus-visible:outline-solid \
     focus-visible:outline-2 focus-visible:outline-oxide focus-visible:outline-offset-2";
const YEAR_CURRENT: &str = "inline-flex min-h-[2rem] min-w-[3.2rem] items-center justify-center \
     rounded-[0.15rem] border border-ink bg-ink px-2 text-card";
const HEAT_FILL: &str =
    "bg-[color-mix(in_srgb,var(--color-oxide)_var(--pants-heat-alpha,0%),var(--color-card))]";
const CELL: &str = "relative block min-w-0 rounded-[0.12rem] border border-hairline/88";
const CELL_OUTSIDE: &str = "border-transparent opacity-25";
const CELL_KWERM: &str = "ring-1 ring-[color-mix(in_srgb,var(--color-patina)_75%,transparent)]";
const CELL_ASYNC: &str = "ring-1 ring-[color-mix(in_srgb,var(--color-patina)_38%,transparent)]";
const DOT: &str = "absolute right-[0.05rem] top-[0.05rem] size-[0.18rem] rounded-full bg-patina";
const INFARCTION: &str = "absolute left-[8%] top-1/2 h-px w-[84%] -rotate-45 \
     bg-[color-mix(in_srgb,var(--color-oxide-hot)_88%,transparent)]";
const WORM: &str =
    "absolute inset-0 flex items-center justify-center text-[0.42rem] leading-none select-none";
const LEGEND_CELL: &str =
    "relative inline-block size-[0.72rem] rounded-[0.12rem] border border-hairline/88";
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

    let calendar = if state.is_none() {
        PantsCalendar::from_messages(&status.messages, now, selected_year)
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
                        <span class=(LEGEND_CELL) aria-hidden="true">
                            <span class=(DOT)></span>
                        </span>
                        "out of town"
                    </span>
                    <span class="inline-flex items-center gap-1">
                        <span class=(LEGEND_CELL) aria-hidden="true">
                            <span class=(INFARCTION)></span>
                        </span>
                        "infarction"
                    </span>
                </div>
            </header>

            if let Some(message) = state {
                <p class=(class!(NOTE, "mt-5 max-w-prose"))>(message)</p>
            } else if let Some(calendar) = calendar {
                <p class=(class!(NOTE, "mt-4"))>
                    (calendar.range_label.as_str())
                </p>
                <div class="mt-6 space-y-7">
                    for person in calendar.people.iter() {
                        <section aria-labelledby=(person.heading_id.as_str())>
                            <header class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
                                <h3
                                    id=(person.heading_id.as_str())
                                    class="font-display text-lg font-semibold"
                                >
                                    (person.display_name)
                                </h3>
                                <p class=(NOTE)>(person.summary.as_str())</p>
                            </header>
                            <div
                                class="mt-2 overflow-x-auto overscroll-x-contain pt-[0.1rem] \
                                     pb-[0.35rem] [direction:rtl]"
                            >
                                <div
                                    class="grid w-full min-w-[34rem] \
                                         grid-cols-[1.45rem_minmax(0,1fr)] \
                                         grid-rows-[1.1rem_auto] gap-x-[0.4rem] [direction:ltr]"
                                >
                                    <div
                                        class="col-start-2 row-start-1 grid items-end \
                                             gap-x-[0.16rem] font-meta text-[0.59rem] \
                                             leading-none whitespace-nowrap text-muted \
                                             sm:gap-x-[0.2rem]"
                                        style=(calendar.column_style.as_str())
                                        aria-hidden="true"
                                    >
                                        for label in calendar.month_labels.iter() {
                                            <span
                                                style=(label.style.as_str())
                                                data-column=(label.column)
                                            >
                                                (label.label.as_str())
                                            </span>
                                        }
                                    </div>
                                    <div
                                        class="col-start-1 row-start-2 grid \
                                             grid-rows-[repeat(7,minmax(0,1fr))] \
                                             items-center self-stretch text-right font-meta \
                                             text-[0.58rem] leading-none text-muted"
                                        aria-hidden="true"
                                    >
                                        <span></span>
                                        <span>"M"</span>
                                        <span></span>
                                        <span>"W"</span>
                                        <span></span>
                                        <span>"F"</span>
                                        <span></span>
                                    </div>
                                    <div
                                        class="col-start-2 row-start-2 grid grid-flow-col \
                                             grid-rows-[repeat(7,minmax(0,1fr))] \
                                             gap-[0.16rem] sm:gap-[0.2rem]"
                                        style=(calendar.chart_style.as_str())
                                        role="group"
                                        aria-label=(person.chart_label.as_str())
                                    >
                                        for cell in person.cells.iter() {
                                            <span
                                                class=(class!(CELL, HEAT_FILL, cell.outside_class, cell.team_class))
                                                data-date=(cell.date.to_string())
                                                data-claims=(cell.claims)
                                                style=(cell.style.as_str())
                                                title=(cell.label.as_str())
                                                aria-label=(cell.label.as_str())
                                                role="img"
                                            >
                                                if cell.out_of_town {
                                                    <span class=(DOT) aria-hidden="true"></span>
                                                }
                                                if cell.infarction {
                                                    <span class=(INFARCTION) aria-hidden="true"></span>
                                                }
                                                if cell.kwerm {
                                                    <span class=(WORM) aria-hidden="true">"🪱"</span>
                                                } else if cell.asynkwerm {
                                                    <span
                                                        class=(class!(WORM, "opacity-45"))
                                                        aria-hidden="true"
                                                    >"🪱"</span>
                                                }
                                            </span>
                                        }
                                    </div>
                                </div>
                            </div>
                        </section>
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
                        for board in calendar.leaderboards.iter() {
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
                    </div>
                    <section
                        class=(class!(BOARD, "mt-4"))
                        aria-labelledby="pants-crew-totals-title"
                    >
                        <div class="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-2">
                            <div>
                                <p class=(META)>"crew totals"</p>
                                <h4
                                    id="pants-crew-totals-title"
                                    class="mt-1 font-display text-base font-semibold"
                                >
                                    "The worm ledger"
                                </h4>
                            </div>
                            <dl class="flex flex-wrap gap-x-8 gap-y-3">
                                <div>
                                    <dt class=(META)>"kwerms"</dt>
                                    <dd class="mt-1 font-display text-xl tabular-nums">
                                        (calendar.crew.kwerms)
                                    </dd>
                                    <dd class=(NOTE)>
                                        (format!(
                                            "{} AM · {} PM · {} {}",
                                            calendar.crew.am_kwerms,
                                            calendar.crew.pm_kwerms,
                                            calendar.crew.kwerm_days,
                                            plural(calendar.crew.kwerm_days, "day", "days")
                                        ))
                                    </dd>
                                </div>
                                <div>
                                    <dt class=(META)>"asynkwerms"</dt>
                                    <dd class="mt-1 font-display text-xl tabular-nums">
                                        (calendar.crew.asynkwerms)
                                    </dd>
                                    <dd class=(NOTE)>"crew claim days without a shared slot"</dd>
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

struct PantsCalendar {
    range_label: String,
    column_style: String,
    chart_style: String,
    month_labels: Vec<MonthLabel>,
    people: Vec<PersonCalendar>,
    leaderboards: Vec<Leaderboard>,
    crew: CrewTotals,
}

impl PantsCalendar {
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
        let stats = yearly_stats(&by_date, year_start, year_end, tracking_start, today);
        let crew = crew_totals(&by_date, year_start, year_end, tracking_start, today);
        let leaderboards = Leaderboard::all(&stats, selected_year);
        let mut people = Vec::with_capacity(PANTS_PARTICIPANTS.len());

        for (participant_index, participant) in PANTS_PARTICIPANTS.iter().enumerate() {
            let cells: Vec<PantsCell> = dates
                .iter()
                .map(|date| {
                    let in_selected_year = date.year() == selected_year;
                    PantsCell::new(
                        *date,
                        in_selected_year.then(|| by_date.get(date)).flatten(),
                        participant_index,
                        tracking_start,
                        today,
                        selected_year,
                    )
                })
                .collect();
            let person_stats = &stats[participant_index];
            people.push(PersonCalendar {
                display_name: participant.display_name,
                heading_id: format!("pants-heatmap-person-{participant_index}"),
                summary: format!(
                    "{} {} · {} claim {} · {} {}",
                    person_stats.claims,
                    plural(person_stats.claims, "claim", "claims"),
                    person_stats.claim_days,
                    plural(person_stats.claim_days, "day", "days"),
                    person_stats.doubles,
                    plural(person_stats.doubles, "double", "doubles")
                ),
                chart_label: format!(
                    "{} Pants Off claims by Eastern day in {selected_year}",
                    participant.display_name
                ),
                cells,
            });
        }

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
            leaderboards,
            crew,
        })
    }
}

struct PersonCalendar {
    display_name: &'static str,
    heading_id: String,
    summary: String,
    chart_label: String,
    cells: Vec<PantsCell>,
}

struct PantsCell {
    date: Date,
    claims: u8,
    out_of_town: bool,
    infarction: bool,
    kwerm: bool,
    asynkwerm: bool,
    outside_class: &'static str,
    team_class: &'static str,
    style: String,
    label: String,
}

impl PantsCell {
    fn new(
        date: Date,
        day: Option<&PantsDay>,
        participant_index: usize,
        tracking_start: Date,
        today: Date,
        selected_year: i16,
    ) -> Self {
        let in_selected_year = date.year() == selected_year;
        let outside = !in_selected_year || date < tracking_start || date > today;
        let participant = day.map(|day| &day.participants[participant_index]);
        let claims = participant.map_or(0, PantsParticipantDay::claims);
        let out_of_town = participant.is_some_and(|day| !day.out_of_town.is_empty());
        let infarction = participant.is_some_and(|day| !day.infarctions.is_empty());
        let kwerm = day.is_some_and(|day| day.kwerm_am || day.kwerm_pm);
        let asynkwerm = day.is_some_and(|day| day.asynkwerm);
        let team_class = if kwerm {
            CELL_KWERM
        } else if asynkwerm {
            CELL_ASYNC
        } else {
            ""
        };
        Self {
            date,
            claims,
            out_of_town,
            infarction,
            kwerm,
            asynkwerm,
            outside_class: if outside { CELL_OUTSIDE } else { "" },
            team_class,
            style: heat_style(claims),
            label: cell_label(
                date,
                day,
                participant,
                outside,
                tracking_start,
                today,
                selected_year,
            ),
        }
    }
}

struct MonthLabel {
    label: String,
    column: usize,
    style: String,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct YearStats {
    participant_index: usize,
    claims: usize,
    claim_days: usize,
    doubles: usize,
    longest_streak: usize,
    streak_start: Option<Date>,
    streak_end: Option<Date>,
}

fn yearly_stats(
    days: &BTreeMap<Date, PantsDay>,
    year_start: Date,
    year_end: Date,
    tracking_start: Date,
    today: Date,
) -> [YearStats; 3] {
    let mut stats = std::array::from_fn(|participant_index| YearStats {
        participant_index,
        ..YearStats::default()
    });
    let start = year_start.max(tracking_start);
    let end = year_end.min(today);
    if start > end {
        return stats;
    }

    let mut date = start;
    let mut current_streaks = [0_usize; 3];
    let mut current_starts = [None; 3];
    loop {
        for (participant_index, person_stats) in stats.iter_mut().enumerate() {
            let claims = days
                .get(&date)
                .map_or(0, |day| day.participants[participant_index].claims());
            person_stats.claims += usize::from(claims);
            if claims > 0 {
                person_stats.claim_days += 1;
                person_stats.doubles += usize::from(claims == 2);
                if current_streaks[participant_index] == 0 {
                    current_starts[participant_index] = Some(date);
                }
                current_streaks[participant_index] += 1;
                if current_streaks[participant_index] >= person_stats.longest_streak {
                    person_stats.longest_streak = current_streaks[participant_index];
                    person_stats.streak_start = current_starts[participant_index];
                    person_stats.streak_end = Some(date);
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
    for person_stats in &stats {
        debug_assert_eq!(
            person_stats.claims,
            person_stats.claim_days + person_stats.doubles
        );
    }
    stats
}

#[derive(Clone, Copy)]
enum LeaderboardMetric {
    Claims,
    Streak,
    Doubles,
}

impl LeaderboardMetric {
    fn value(self, stats: &YearStats) -> usize {
        match self {
            Self::Claims => stats.claims,
            Self::Streak => stats.longest_streak,
            Self::Doubles => stats.doubles,
        }
    }
}

struct Leaderboard {
    heading_id: String,
    title: &'static str,
    entries: Vec<LeaderboardEntry>,
}

impl Leaderboard {
    fn all(stats: &[YearStats; 3], year: i16) -> Vec<Self> {
        [
            (LeaderboardMetric::Claims, "Most claims", "claims"),
            (LeaderboardMetric::Streak, "Longest claim streak", "streak"),
            (LeaderboardMetric::Doubles, "Most doubles", "doubles"),
        ]
        .into_iter()
        .map(|(metric, title, slug)| Self {
            heading_id: format!("pants-board-{year}-{slug}"),
            title,
            entries: ranked_entries(stats, metric),
        })
        .collect()
    }
}

struct LeaderboardEntry {
    rank: String,
    display_name: &'static str,
    value: String,
    detail: String,
}

fn ranked_entries(all_stats: &[YearStats; 3], metric: LeaderboardMetric) -> Vec<LeaderboardEntry> {
    let mut ordered: Vec<&YearStats> = all_stats.iter().collect();
    ordered.sort_by_key(|stats| {
        (
            std::cmp::Reverse(metric.value(stats)),
            stats.participant_index,
        )
    });
    let mut previous_value = None;
    let mut rank = 0;

    ordered
        .into_iter()
        .enumerate()
        .map(|(position, stats)| {
            let metric_value = metric.value(stats);
            if previous_value != Some(metric_value) {
                rank = position + 1;
                previous_value = Some(metric_value);
            }
            let tied = metric_value > 0
                && all_stats
                    .iter()
                    .filter(|candidate| metric.value(candidate) == metric_value)
                    .count()
                    > 1;
            LeaderboardEntry {
                rank: if metric_value == 0 {
                    "—".to_string()
                } else if tied {
                    format!("T{rank}")
                } else {
                    rank.to_string()
                },
                display_name: PANTS_PARTICIPANTS[stats.participant_index].display_name,
                value: metric_value_label(metric, metric_value),
                detail: metric_detail(metric, stats),
            }
        })
        .collect()
}

fn metric_value_label(metric: LeaderboardMetric, value: usize) -> String {
    match metric {
        LeaderboardMetric::Claims => {
            format!("{value} {}", plural(value, "claim", "claims"))
        }
        LeaderboardMetric::Streak => {
            format!("{value} {}", plural(value, "day", "days"))
        }
        LeaderboardMetric::Doubles => {
            format!("{value} {}", plural(value, "double", "doubles"))
        }
    }
}

fn metric_detail(metric: LeaderboardMetric, stats: &YearStats) -> String {
    match metric {
        LeaderboardMetric::Claims => format!(
            "{} claim {}",
            stats.claim_days,
            plural(stats.claim_days, "day", "days")
        ),
        LeaderboardMetric::Streak => match (stats.streak_start, stats.streak_end) {
            (Some(start), Some(end)) => {
                format!("{} – {}", format_compact(start), format_compact(end))
            }
            _ => "no claim streak".to_string(),
        },
        LeaderboardMetric::Doubles => format!(
            "{} {} on double days",
            stats.doubles * 2,
            plural(stats.doubles * 2, "claim", "claims")
        ),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CrewTotals {
    kwerms: usize,
    am_kwerms: usize,
    pm_kwerms: usize,
    kwerm_days: usize,
    asynkwerms: usize,
}

fn crew_totals(
    days: &BTreeMap<Date, PantsDay>,
    year_start: Date,
    year_end: Date,
    tracking_start: Date,
    today: Date,
) -> CrewTotals {
    let start = year_start.max(tracking_start);
    let end = year_end.min(today);
    if start > end {
        return CrewTotals::default();
    }
    days.range(start..=end)
        .fold(CrewTotals::default(), |mut totals, (_, day)| {
            totals.am_kwerms += usize::from(day.kwerm_am);
            totals.pm_kwerms += usize::from(day.kwerm_pm);
            totals.kwerms += usize::from(day.kwerm_count());
            totals.kwerm_days += usize::from(day.kwerm_count() > 0);
            totals.asynkwerms += usize::from(day.asynkwerm);
            totals
        })
}

fn inclusive_dates(start: Date, end: Date) -> Option<Vec<Date>> {
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

fn heat_style(claims: u8) -> String {
    let alpha = match claims {
        0 => 0,
        1 => 34,
        _ => 82,
    };
    format!("--pants-heat-alpha: {alpha}%")
}

fn cell_label(
    date: Date,
    day: Option<&PantsDay>,
    participant: Option<&PantsParticipantDay>,
    outside: bool,
    tracking_start: Date,
    today: Date,
    selected_year: i16,
) -> String {
    let date_label = format_long(date);
    if date.year() != selected_year {
        return format!("{date_label}: outside {selected_year}");
    }
    if outside {
        return if date > today {
            format!("{date_label}: after today")
        } else if date < tracking_start {
            format!("{date_label}: before tracking")
        } else {
            format!("{date_label}: outside the calendar")
        };
    }
    let mut facts = Vec::new();
    match participant {
        Some(person) => {
            let slots: Vec<&str> = [
                person.first(PantsSlot::Am).map(|_| "6:07 AM"),
                person.first(PantsSlot::Pm).map(|_| "6:07 PM"),
            ]
            .into_iter()
            .flatten()
            .collect();
            if slots.is_empty() {
                facts.push("0 claims".to_string());
            } else if slots.len() == 2 {
                facts.push("2 claims — double (6:07 AM and 6:07 PM)".to_string());
            } else {
                facts.push(format!("1 claim ({})", slots[0]));
            }
            if !person.out_of_town.is_empty() {
                facts.push(format!(
                    "out of town at {}",
                    moment_clocks(&person.out_of_town)
                ));
            }
            if !person.infarctions.is_empty() {
                facts.push(format!(
                    "infarction at {}",
                    moment_clocks(&person.infarctions)
                ));
            }
        }
        None => facts.push("0 claims".to_string()),
    }
    if let Some(day) = day {
        let mut slots = Vec::new();
        if day.kwerm_am {
            slots.push("AM");
        }
        if day.kwerm_pm {
            slots.push("PM");
        }
        if !slots.is_empty() {
            facts.push(format!("kwerm ({})", slots.join(" and ")));
        } else if day.asynkwerm {
            facts.push("asynkwerm".to_string());
        }
    }
    format!("{date_label}: {}", facts.join("; "))
}

fn moment_clocks(moments: &[benjisponge::data::podrick_models::ClassifiedPantsMessage]) -> String {
    moments
        .iter()
        .map(|moment| moment.clock_label())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_short(date: Date) -> String {
    date.strftime("%b %-d, %Y").to_string()
}

fn format_long(date: Date) -> String {
    date.strftime("%A, %b %-d, %Y").to_string()
}

fn format_compact(date: Date) -> String {
    date.strftime("%b %-d").to_string()
}

const fn plural<'a>(value: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if value == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::*;
    use benjisponge::data::podrick_models::PANTS_CHANNEL_ID;
    use jiff::Timestamp;

    fn epoch(instant: &str) -> i64 {
        instant.parse::<Timestamp>().unwrap().as_second()
    }

    fn message(id: &str, participant: usize, instant: &str) -> PodrickPantsMessage {
        PodrickPantsMessage {
            id: id.to_string(),
            message_id: id.to_string(),
            channel_id: PANTS_CHANNEL_ID.to_string(),
            author_id: PANTS_PARTICIPANTS[participant].author_id.to_string(),
            posted_at: epoch(instant),
        }
    }

    fn calendar(messages: &[PodrickPantsMessage], year: i16) -> PantsCalendar {
        PantsCalendar::from_messages(messages, epoch("2026-07-28T16:00:00Z"), year).unwrap()
    }

    #[test]
    fn ordinary_year_is_53_complete_weeks_with_january_through_december_labels() {
        let calendar = calendar(&[message("101", 0, "2025-01-01T11:07:00Z")], 2025);
        assert_eq!(calendar.people.len(), 3);
        assert_eq!(calendar.people[0].cells.len(), 371);
        assert_eq!(calendar.people[0].cells[0].date.to_string(), "2024-12-29");
        assert_eq!(
            calendar.people[0].cells.last().unwrap().date.to_string(),
            "2026-01-03"
        );
        assert_eq!(calendar.month_labels.len(), 12);
        assert_eq!(calendar.month_labels[0].label, "Jan");
        assert_eq!(calendar.month_labels[0].column, 1);
        assert_eq!(calendar.month_labels[11].label, "Dec");
        assert!(calendar.chart_style.contains("repeat(53,"));
    }

    #[test]
    fn leap_year_beginning_saturday_uses_54_weeks() {
        let tracking_start = Date::new(2028, 1, 1).unwrap();
        let calendar =
            PantsCalendar::new(Vec::new(), tracking_start, tracking_start, 2028).unwrap();
        assert_eq!(calendar.people[0].cells.len(), 378);
        assert_eq!(calendar.people[0].cells[0].date.to_string(), "2027-12-26");
        assert_eq!(
            calendar.people[0].cells.last().unwrap().date.to_string(),
            "2029-01-06"
        );
        assert!(calendar.chart_style.contains("repeat(54,"));
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
        let calendar = calendar(&messages, 2026);
        let cell = calendar.people[0]
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
        let calendar = calendar(&messages, 2025);
        let padding = &calendar.people[0].cells[0];
        assert_eq!(padding.date.to_string(), "2024-12-29");
        assert_eq!(padding.claims, 0);
        assert!(!padding.kwerm);
        assert!(padding.label.contains("outside 2025"));
        assert_eq!(calendar.leaderboards[0].entries[0].value, "1 claim");
    }

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
        let days = aggregate_pants_messages(&messages);
        let by_date: BTreeMap<_, _> = days.into_iter().map(|day| (day.date, day)).collect();
        let start = Date::new(2026, 1, 1).unwrap();
        let end = Date::new(2026, 12, 31).unwrap();
        let stats = yearly_stats(&by_date, start, end, start, Date::new(2026, 7, 28).unwrap());
        assert_eq!(stats[0].claims, 5);
        assert_eq!(stats[0].claim_days, 4);
        assert_eq!(stats[0].doubles, 1);
        assert_eq!(stats[0].longest_streak, 2);
        assert_eq!(stats[0].streak_start.unwrap().to_string(), "2026-01-04");
        assert_eq!(stats[0].streak_end.unwrap().to_string(), "2026-01-05");
    }

    #[test]
    fn streaks_reset_at_the_year_boundary() {
        let messages = [
            message("101", 0, "2025-12-31T11:07:00Z"),
            message("102", 0, "2026-01-01T11:07:00Z"),
            message("103", 0, "2026-01-02T11:07:00Z"),
        ];
        let days = aggregate_pants_messages(&messages);
        let by_date: BTreeMap<_, _> = days.into_iter().map(|day| (day.date, day)).collect();
        let start = Date::new(2026, 1, 1).unwrap();
        let stats = yearly_stats(
            &by_date,
            start,
            Date::new(2026, 12, 31).unwrap(),
            Date::new(2025, 12, 31).unwrap(),
            Date::new(2026, 7, 28).unwrap(),
        );
        assert_eq!(stats[0].longest_streak, 2);
        assert_eq!(stats[0].streak_start.unwrap().to_string(), "2026-01-01");
    }

    #[test]
    fn leaderboards_use_competition_ranks_and_do_not_rank_zeroes() {
        let stats = [
            YearStats {
                participant_index: 0,
                claims: 2,
                ..YearStats::default()
            },
            YearStats {
                participant_index: 1,
                claims: 2,
                ..YearStats::default()
            },
            YearStats {
                participant_index: 2,
                claims: 1,
                ..YearStats::default()
            },
        ];
        let claims = ranked_entries(&stats, LeaderboardMetric::Claims);
        assert_eq!(claims[0].rank, "T1");
        assert_eq!(claims[1].rank, "T1");
        assert_eq!(claims[2].rank, "3");

        let doubles = ranked_entries(&stats, LeaderboardMetric::Doubles);
        assert!(doubles.iter().all(|entry| entry.rank == "—"));
    }

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
        let days = aggregate_pants_messages(&messages);
        let by_date: BTreeMap<_, _> = days.into_iter().map(|day| (day.date, day)).collect();
        let start = Date::new(2026, 1, 1).unwrap();
        let totals = crew_totals(
            &by_date,
            start,
            Date::new(2026, 12, 31).unwrap(),
            start,
            Date::new(2026, 7, 28).unwrap(),
        );
        assert_eq!(totals.kwerms, 2);
        assert_eq!(totals.am_kwerms, 1);
        assert_eq!(totals.pm_kwerms, 1);
        assert_eq!(totals.kwerm_days, 1);
        assert_eq!(totals.asynkwerms, 1);
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
