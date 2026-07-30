//! One participant's calendar-year heatmap.

use benjisponge::data::podrick_models::{
    ClassifiedPantsMessage, PantsDay, PantsParticipant, PantsParticipantDay, PantsSlot,
};
use jiff::civil::Date;
use topcoat::{
    Result,
    view::{class, component, view},
};

use super::{
    CELL_ASYNC, DOT, HEAT_FILL, INFARCTION_A, INFARCTION_B, MonthLabel, WORM, YearWindow,
    util::{NOTE, format_long, plural},
};

const CELL: &str = "relative block min-w-0 rounded-[0.12rem] border border-hairline/88";
const CELL_OUTSIDE: &str = "border-transparent opacity-25";
const CELL_KWERM: &str = "ring-1 ring-[color-mix(in_srgb,var(--color-patina)_75%,transparent)]";

#[component]
pub(super) async fn person_heatmap(
    person: &PersonCalendar,
    column_style: &str,
    chart_style: &str,
    month_labels: &[MonthLabel],
) -> Result {
    view! {
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
                        style=(column_style)
                        aria-hidden="true"
                    >
                        for label in month_labels.iter() {
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
                        style=(chart_style)
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
                                    <span class=(INFARCTION_A) aria-hidden="true"></span>
                                    <span class=(INFARCTION_B) aria-hidden="true"></span>
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
}

pub(super) struct PersonCalendar {
    pub display_name: &'static str,
    pub heading_id: String,
    pub summary: String,
    pub chart_label: String,
    pub cells: Vec<PantsCell>,
}

impl PersonCalendar {
    pub fn build(
        participant: &PantsParticipant,
        participant_index: usize,
        dates: &[Date],
        window: &YearWindow<'_>,
    ) -> Self {
        let mut claims = 0_usize;
        let mut claim_days = 0_usize;
        let mut doubles = 0_usize;
        let cells: Vec<PantsCell> = dates
            .iter()
            .map(|date| {
                let in_selected_year = date.year() == window.selected_year;
                let day = in_selected_year.then(|| window.by_date.get(date)).flatten();
                let cell = PantsCell::new(
                    *date,
                    day,
                    participant_index,
                    window.tracking_start,
                    window.today,
                    window.selected_year,
                );
                if in_selected_year
                    && *date >= window.tracking_start
                    && *date <= window.today
                    && cell.claims > 0
                {
                    claims += usize::from(cell.claims);
                    claim_days += 1;
                    doubles += usize::from(cell.claims == 2);
                }
                cell
            })
            .collect();
        Self {
            display_name: participant.display_name,
            heading_id: format!("pants-heatmap-person-{participant_index}"),
            summary: format!(
                "{} {} · {} claim {} · {} {}",
                claims,
                plural(claims, "claim", "claims"),
                claim_days,
                plural(claim_days, "day", "days"),
                doubles,
                plural(doubles, "double", "doubles")
            ),
            chart_label: format!(
                "{} Pants Off claims by Eastern day in {}",
                participant.display_name, window.selected_year
            ),
            cells,
        }
    }
}

pub(super) struct PantsCell {
    pub date: Date,
    pub claims: u8,
    pub out_of_town: bool,
    pub infarction: bool,
    pub kwerm: bool,
    pub asynkwerm: bool,
    pub outside_class: &'static str,
    pub team_class: &'static str,
    pub style: String,
    pub label: String,
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
            style: heat_style(claims, out_of_town),
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

fn heat_style(claims: u8, out_of_town: bool) -> String {
    // Out-of-town-only days share the one-claim fill so the patina dot reads
    // against the same oxide wash as a normal claim.
    let alpha = match (claims, out_of_town) {
        (0, false) => 0,
        (0 | 1, _) => 34,
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

fn moment_clocks(moments: &[ClassifiedPantsMessage]) -> String {
    moments
        .iter()
        .map(|moment| moment.clock_label())
        .collect::<Vec<_>>()
        .join(", ")
}
