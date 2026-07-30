//! A local-date calendar of lifting volume points. Logged days open a shared
//! popover whose body is a Topcoat shard — the heatmap SSR stays light; the
//! day's lifts and muscle maps load on demand when the reader hovers or
//! clicks. `heatmap-preview.js` handles hover/pin chrome; the shard owns the
//! data. Arguments to the shard are untrusted and validated server-side.

use std::collections::BTreeMap;

use jiff::{ToSpan, civil::Date};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    runtime::{Event, shard},
    view::{class, component, view},
};

use super::{
    META_LABEL,
    archive::store::FitnessStore,
    data::CalendarDay,
    format::{format_duration, plural},
    muscles,
    results::workout_url,
};

const WEEK_COUNT: usize = 53;
const DAYS_PER_WEEK: usize = 7;
const CELL_COUNT: usize = WEEK_COUNT * DAYS_PER_WEEK;
const PREVIEW_POPOVER_ID: &str = "heatmap-day-preview";

const HEAT_NOTE: &str = "font-meta text-[0.7rem] leading-[1.55] text-muted";
const HEAT_FILL: &str =
    "bg-[color-mix(in_srgb,var(--color-oxide)_var(--fitness-heat-alpha,0%),var(--color-card))]";
const LEGEND_CELL: &str = "w-[0.625rem] h-[0.625rem] sm:w-[0.72rem] sm:h-[0.72rem] \
     rounded-[0.12rem] border border-hairline/88";
const CELL: &str = "block rounded-[0.12rem] border \
     transition-[background-color,border-color,box-shadow,transform] duration-[140ms] ease-[ease]";
const CELL_BORDER: &str = "border-hairline/88";
const CELL_BORDER_ZERO: &str = "border-dashed \
     border-[color-mix(in_srgb,var(--color-oxide)_55%,var(--color-hairline))]";
const CELL_HOVER: &str = "hover:border-oxide \
     hover:shadow-[0_0_0_1px_color-mix(in_srgb,var(--color-oxide)_25%,transparent)] \
     hover:-translate-y-px focus-visible:z-[1] focus-visible:outline-solid \
     focus-visible:outline-2 focus-visible:outline-oxide focus-visible:outline-offset-2";
const CELL_BUTTON: &str = "appearance-none p-0 size-full cursor-pointer";
const PREVIEW_WORKOUT_TITLE: &str = "font-display text-[0.95rem] font-semibold leading-[1.25] \
     text-ink decoration-oxide/45 decoration-1 underline-offset-[0.18em] \
     hover:text-oxide hover:decoration-current focus-visible:text-oxide \
     focus-visible:decoration-current";
const PREVIEW_EXERCISE: &str = "font-meta text-[0.68rem] leading-[1.45] text-ink2";
const PREVIEW_LOG_LINK: &str = "mt-[0.75rem] inline-block font-meta text-[0.72rem] text-oxide \
     decoration-oxide/45 decoration-1 underline-offset-[0.18em] hover:decoration-current \
     focus-visible:decoration-current";

const HEATMAP_PREVIEW_JS: Asset = asset!("./heatmap-preview.js");

/// Volume calendar. `link_query` carries the log page's active filters
/// (canonical, minus `from`/`to`/`page`) into the preview's day-log link.
/// Day lift bodies are not embedded here — they load through
/// [`day_preview_shard`] when `preview_day` is set.
#[component]
pub(super) async fn calendar_heatmap(
    days: Vec<CalendarDay>,
    #[default(String::new())] link_query: String,
    #[default(false)] filtered: bool,
) -> Result {
    let Some(calendar) = Calendar::from_days(&days) else {
        return view! {
            <section aria-labelledby="fitness-heatmap-title">
                <header
                    class="flex flex-wrap items-end justify-between gap-y-[0.8rem] gap-x-5"
                >
                    <div>
                        <p class=(META_LABEL)>"training volume"</p>
                        <h2
                            id="fitness-heatmap-title"
                            class="font-display text-2xl font-semibold"
                        >
                            "Volume points"
                        </h2>
                    </div>
                </header>
                <p class=(class!(HEAT_NOTE, "mt-[0.8rem]"))>
                    if filtered {
                        "No logged days match these filters."
                    } else {
                        "No lifting days are available yet."
                    }
                </p>
            </section>
        };
    };

    let ending = format_short(calendar.latest);
    let start = format_short(calendar.latest - 53.weeks());
    let subtitle = if filtered {
        format!("{start} - {ending} · matching sets only")
    } else {
        format!("{start} - {ending}")
    };
    let navigation_label = if filtered {
        format!(
            "Volume points from sets matching the active filters, by day, for the 53 weeks ending {ending}. {} matching days open a preview of that day's lifts.",
            calendar.logged_days,
        )
    } else {
        format!(
            "Volume points by day for the 53 weeks ending {ending}. {} logged days open a preview of that day's lifts.",
            calendar.logged_days,
        )
    };
    let legend_styles: Vec<String> = (0..=4).map(heat_style).collect();
    let shard_link_query = link_query.clone();

    view! {
        <section aria-labelledby="fitness-heatmap-title" data-heatmap-previews="">
            signal preview_day = String::new();
            <input
                type="hidden"
                data-heatmap-day-input=""
                :value=$(preview_day.get())
                @input=$(|e: Event| preview_day.set(e.target.value))
            />
            <header class="flex flex-wrap items-end justify-between gap-y-[0.8rem] gap-x-5">
                <div>
                    <p class=(META_LABEL)>"training volume"</p>
                    <h2
                        id="fitness-heatmap-title"
                        class="font-display text-2xl font-semibold"
                    >
                        "Volume points"
                    </h2>
                    <p class=(class!(HEAT_NOTE, "mt-[0.3rem]"))>(subtitle.as_str())</p>
                </div>
                <div
                    class="inline-flex items-center gap-[0.22rem] font-meta text-[0.61rem] \
                         leading-none uppercase text-muted"
                    aria-label="Volume-point intensity: 1 to 24, 25 to 44, 45 to 64, and 65 or more"
                >
                    <span class="mr-[0.12rem]">"less"</span>
                    for style in legend_styles.iter() {
                        <span
                            class=(class!(LEGEND_CELL, HEAT_FILL))
                            style=(style.as_str())
                            aria-hidden="true"
                        >

                        </span>
                    }
                    <span class="ml-[0.12rem]">"more"</span>
                </div>
            </header>

            <div
                class="mt-[0.9rem] overflow-x-auto overscroll-x-contain pt-[0.1rem] \
                     pb-[0.45rem] [direction:rtl]"
            >
                <div
                    class="grid w-full min-w-[34rem] grid-cols-[1.45rem_minmax(0,1fr)] \
                         grid-rows-[1.1rem_auto] gap-x-[0.4rem] [direction:ltr]"
                >
                    <div
                        class="col-start-2 row-start-1 grid \
                             grid-cols-[repeat(53,minmax(0,1fr))] gap-x-[0.16rem] items-end \
                             font-meta text-[0.59rem] leading-none whitespace-nowrap \
                             text-muted sm:gap-x-[0.2rem]"
                        aria-hidden="true"
                    >
                        for label in calendar.month_labels.iter() {
                            <span style=(label.style.as_str())>
                                (label.label.as_str())
                            </span>
                        }
                    </div>
                    <div
                        class="col-start-1 row-start-2 grid \
                             grid-rows-[repeat(7,minmax(0,1fr))] items-center self-stretch \
                             text-right font-meta text-[0.58rem] leading-none text-muted"
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
                    <nav
                        class="col-start-2 row-start-2 grid \
                             grid-cols-[repeat(53,minmax(0,1fr))] \
                             grid-rows-[repeat(7,minmax(0,1fr))] grid-flow-col \
                             gap-[0.16rem] aspect-[53/7] sm:gap-[0.2rem]"
                        aria-label=(navigation_label.as_str())
                    >
                        for cell in calendar.cells.iter() {
                            day_cell(cell: cell)
                        }
                    </nav>
                </div>
            </div>

            <div
                id=(PREVIEW_POPOVER_ID)
                class="inline-popover-panel"
                popover="auto"
                data-heatmap-panel=""
            >
                <button
                    type="button"
                    class="inline-popover-close"
                    popovertarget=(PREVIEW_POPOVER_ID)
                    popovertargetaction="hide"
                    aria-label="Close preview"
                >"×"</button>
                day_preview_shard(
                    date: $(preview_day.get()),
                    link_query: $(shard_link_query.to_owned())
                )
            </div>
            <script type="module" src=(HEATMAP_PREVIEW_JS)></script>
        </section>
    }
}

/// On-demand body for a heatmap day popover. `date` and `link_query` arrive
/// from the browser and are validated before touching the archive.
#[shard]
async fn day_preview_shard(cx: &Cx, date: String, link_query: String) -> Result {
    if date.is_empty() {
        return view! {};
    }
    let Ok(parsed) = date.parse::<Date>() else {
        return view! {
            <p class=(HEAT_NOTE)>"That day could not be loaded."</p>
        };
    };
    let date = parsed.to_string();
    let link_query = sanitize_link_query(&link_query);
    let href = if link_query.is_empty() {
        format!("/lifting/log?from={date}&to={date}#set-log")
    } else {
        format!("/lifting/log?{link_query}&from={date}&to={date}#set-log")
    };

    let store = app_context::<FitnessStore>(cx);
    let snapshot = match store.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(_) => {
            return view! {
                <p class=(HEAT_NOTE)>"The workout archive is unavailable right now."</p>
            };
        }
    };
    let summaries = snapshot.workouts_on_date(&date);
    if summaries.is_empty() {
        return view! {
            <span class="inline-popover-kicker">(format_long(parsed).as_str())</span>
            <p class=(HEAT_NOTE)>"No lifts are stored for this day."</p>
            <a class=(PREVIEW_LOG_LINK) href=(href.as_str())>"view day in log →"</a>
        };
    }

    let weights = snapshot.exercise_weight_map();
    let mut volume_points = 0_u32;
    let mut workouts = Vec::with_capacity(summaries.len());
    for summary in summaries {
        volume_points = volume_points.saturating_add(summary.volume_points);
        let involvement = muscles::involvement_for_exercises(
            summary.exercises.iter().map(String::as_str),
            weights,
        );
        workouts.push(ShardWorkout {
            title: summary.title,
            href: workout_url(&summary.path),
            duration: format_duration(summary.duration_seconds),
            set_count: summary.set_count,
            exercises: summary.exercises,
            involvement,
        });
    }
    let heading = format_long(parsed);
    let points_label = format!(
        "{volume_points} volume {}",
        if volume_points == 1 {
            "point"
        } else {
            "points"
        }
    );

    view! {
        <span class="inline-popover-kicker">(heading.as_str())</span>
        <span class="inline-popover-detail">(points_label.as_str())</span>
        <div class="space-y-[0.75rem]">
            for workout in workouts.iter() {
                workout_block(workout: workout)
            }
        </div>
        <a class=(PREVIEW_LOG_LINK) href=(href.as_str())>"view day in log →"</a>
    }
}

#[component]
async fn workout_block(workout: &ShardWorkout) -> Result {
    let exercises = workout.exercises.join(" · ");
    let meta = format!(
        "{} · {} {}",
        workout.duration,
        workout.set_count,
        plural(workout.set_count, "set", "sets"),
    );
    let title_label = format!("Open {} workout", workout.title);
    view! {
        <article class="border-t border-hairline pt-[0.65rem] first:border-t-0 first:pt-0">
            <header class="flex items-baseline justify-between gap-3">
                <a
                    class=(PREVIEW_WORKOUT_TITLE)
                    href=(workout.href.as_str())
                    aria-label=(title_label.as_str())
                >
                    (workout.title.as_str())
                </a>
                <span class="flex-none font-meta text-[0.62rem] leading-[1.4] text-muted">
                    (meta.as_str())
                </span>
            </header>
            if !exercises.is_empty() {
                <p class=(class!(PREVIEW_EXERCISE, "mt-[0.3rem]"))>(exercises.as_str())</p>
            }
            if !workout.involvement.is_empty() {
                muscles::muscle_map_compact(involvement: &workout.involvement)
            }
        </article>
    }
}

struct ShardWorkout {
    title: String,
    href: String,
    duration: String,
    set_count: usize,
    exercises: Vec<String>,
    involvement: muscles::MuscleInvolvement,
}

#[component]
async fn day_cell(cell: &HeatmapCell) -> Result {
    if let Some(date_key) = &cell.date_key {
        // Named CSS anchor so the shared day popover can sit beside this
        // cell — `heatmap-preview.js` points `position-anchor` at it on show.
        let style = format!("{}; anchor-name: --heatmap-day-{date_key};", cell.style);
        view! {
            <button
                type="button"
                class=(class!(CELL, HEAT_FILL, cell.border, CELL_HOVER, CELL_BUTTON))
                popovertarget=(PREVIEW_POPOVER_ID)
                popovertargetaction="show"
                data-heatmap-trigger=""
                data-heatmap-date=(date_key.as_str())
                aria-label=(cell.label.as_str())
                style=(style.as_str())
            >

            </button>
        }
    } else {
        view! {
            <span
                class=(class!(CELL, HEAT_FILL, cell.border))
                title=(cell.label.as_str())
                aria-hidden="true"
                style=(cell.style.as_str())
            >

            </span>
        }
    }
}

/// Query strings we echo into the day-log link must stay filter-shaped —
/// shard args are attacker-controlled.
fn sanitize_link_query(raw: &str) -> &str {
    if raw.is_empty() {
        return "";
    }
    let ok = raw.bytes().all(|byte| {
        matches!(
            byte,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'='
                | b'&'
                | b'%'
                | b'.'
                | b'-'
                | b'_'
                | b'+'
        )
    });
    if ok { raw } else { "" }
}

struct Calendar {
    latest: Date,
    logged_days: usize,
    cells: Vec<HeatmapCell>,
    month_labels: Vec<MonthLabel>,
}

impl Calendar {
    fn from_days(days: &[CalendarDay]) -> Option<Self> {
        let mut points_by_day = BTreeMap::new();
        for day in days {
            let date: Date = day.date.parse().ok()?;
            let points = points_by_day.entry(date).or_insert(0_u32);
            *points = points.saturating_add(day.volume_points);
        }
        let latest = *points_by_day.last_key_value()?.0;
        let end_offset =
            DAYS_PER_WEEK as i64 - 1 - i64::from(latest.weekday().to_sunday_zero_offset());
        let end = latest.checked_add(end_offset.days()).ok()?;
        let start = end.checked_add((-(CELL_COUNT as i64 - 1)).days()).ok()?;
        let mut cells = Vec::with_capacity(CELL_COUNT);
        let mut logged_days = 0;
        for offset in 0..CELL_COUNT {
            let date = start + (offset as i64).days();
            let points = points_by_day.get(&date).copied().unwrap_or(0);
            let has_lift = points_by_day.contains_key(&date);
            if has_lift {
                logged_days += 1;
            }
            cells.push(HeatmapCell::new(date, points, has_lift));
        }
        let month_labels = MonthLabel::from_cells(&cells);
        Some(Self {
            latest,
            logged_days,
            cells,
            month_labels,
        })
    }
}

struct HeatmapCell {
    date: Date,
    date_key: Option<String>,
    border: &'static str,
    label: String,
    style: String,
}

impl HeatmapCell {
    fn new(date: Date, points: u32, has_lift: bool) -> Self {
        let intensity = intensity(points);
        let border = if has_lift && points == 0 {
            CELL_BORDER_ZERO
        } else {
            CELL_BORDER
        };
        let date_label = format_long(date);
        let points_label = format!(
            "{points} volume {}",
            if points == 1 { "point" } else { "points" }
        );
        let label = if has_lift {
            format!("{date_label}: {points_label}. Preview lifts from this day.")
        } else {
            format!("{date_label}: no volume points")
        };
        Self {
            date,
            date_key: has_lift.then(|| date.to_string()),
            border,
            label,
            style: heat_style(intensity),
        }
    }
}

struct MonthLabel {
    label: String,
    style: String,
}

impl MonthLabel {
    fn from_cells(cells: &[HeatmapCell]) -> Vec<Self> {
        let mut labels = Vec::new();
        let mut last_column = None;
        for (index, cell) in cells.iter().enumerate() {
            let column = index / DAYS_PER_WEEK;
            let starts_month = column == 0 || cell.date.day() == 1;
            let has_room = last_column.is_none_or(|previous| column >= previous + 3);
            if starts_month && has_room {
                labels.push(Self {
                    label: cell.date.strftime("%b").to_string(),
                    style: format!("grid-column: {}", column + 1),
                });
                last_column = Some(column);
            }
        }
        labels
    }
}

fn intensity(points: u32) -> u8 {
    match points {
        0 => 0,
        1..=24 => 1,
        25..=44 => 2,
        45..=64 => 3,
        _ => 4,
    }
}

fn heat_style(intensity: u8) -> String {
    let alpha = match intensity {
        0 => 0,
        1 => 18,
        2 => 36,
        3 => 62,
        _ => 92,
    };
    format!("--fitness-heat-alpha: {alpha}%")
}

fn format_short(date: Date) -> String {
    date.strftime("%b %-d, %Y").to_string()
}

fn format_long(date: Date) -> String {
    date.strftime("%A, %b %-d, %Y").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: &str, volume_points: u32) -> CalendarDay {
        CalendarDay {
            date: date.to_string(),
            volume_points,
        }
    }

    #[test]
    fn grid_is_53_complete_sunday_to_saturday_weeks_anchored_to_latest_day() {
        let calendar = Calendar::from_days(&[day("2026-07-21", 42)]).expect("calendar");

        assert_eq!(calendar.cells.len(), 371);
        assert_eq!(calendar.latest.to_string(), "2026-07-21");
        assert_eq!(calendar.month_labels[0].label, "Jul");
        assert_eq!(calendar.month_labels[0].style, "grid-column: 1");
        assert!(
            calendar
                .month_labels
                .iter()
                .any(|label| label.label == "Jan")
        );
        assert_eq!(calendar.cells[0].date.to_string(), "2025-07-20");
        assert_eq!(
            calendar.cells[0].label,
            "Sunday, Jul 20, 2025: no volume points"
        );
        assert_eq!(
            calendar.cells[366].label,
            "Tuesday, Jul 21, 2026: 42 volume points. Preview lifts from this day."
        );
        assert_eq!(calendar.cells[366].date_key.as_deref(), Some("2026-07-21"));
        assert!(calendar.cells[367].date_key.is_none());
    }

    #[test]
    fn intensity_bands_are_fixed_at_their_inclusive_edges() {
        assert_eq!(intensity(0), 0);
        assert_eq!(intensity(1), 1);
        assert_eq!(intensity(24), 1);
        assert_eq!(intensity(25), 2);
        assert_eq!(intensity(44), 2);
        assert_eq!(intensity(45), 3);
        assert_eq!(intensity(64), 3);
        assert_eq!(intensity(65), 4);
        assert_eq!(intensity(u32::MAX), 4);
    }

    #[test]
    fn duplicate_calendar_days_sum_without_losing_their_link() {
        let calendar =
            Calendar::from_days(&[day("2024-02-29", 20), day("2024-02-29", 25)]).expect("calendar");
        let leap_day = calendar
            .cells
            .iter()
            .find(|cell| cell.date_key.as_deref() == Some("2024-02-29"))
            .expect("leap day cell");
        assert!(leap_day.label.contains("45 volume points"));
        assert_eq!(leap_day.style, heat_style(3));
    }

    #[test]
    fn link_query_sanitizer_rejects_schemes_and_junk() {
        assert_eq!(
            sanitize_link_query("movement=squat-type"),
            "movement=squat-type"
        );
        assert_eq!(sanitize_link_query("javascript:alert(1)"), "");
        assert_eq!(sanitize_link_query("a b"), "");
    }
}
