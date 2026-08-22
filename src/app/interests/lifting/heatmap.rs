//! A local-date calendar of lifting volume points. Logged days open a shared
//! popover whose body is a Topcoat shard — the heatmap SSR stays light; the
//! day's lifts and muscle maps load on demand when the reader hovers or
//! clicks. `heatmap-preview.js` handles hover/pin chrome; the shard owns the
//! data. Arguments to the shard are untrusted and validated server-side.

use std::collections::BTreeMap;

use jiff::{Timestamp, ToSpan, civil::Date};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    runtime::{Event, shard},
    view::{class, component, view},
};

use super::{
    META_LABEL,
    archive::{eastern, store::FitnessStore},
    data::CalendarDay,
    format::{format_duration, plural},
    interruptions, muscles,
    results::workout_url,
};
use benjisponge::data::{Data, fitness_models::Interruption, running_models::RunningActivity};

use crate::app::interests::running;

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
const CELL_BUTTON: &str = "appearance-none p-0 size-full cursor-pointer \
     relative overflow-hidden flex items-center justify-center";
const CELL_EMOJI: &str = "pointer-events-none select-none text-[0.55rem] leading-none \
     sm:text-[0.62rem]";
const CELL_RUN_MARKER: &str = "pointer-events-none absolute right-[0.06rem] bottom-[0.06rem] \
     size-[0.2rem] border-r-2 border-b-2 border-patina sm:size-[0.24rem]";
const PREVIEW_INTERRUPTION: &str = "mt-[0.55rem] font-meta text-[0.72rem] leading-[1.45] text-ink2";
const PREVIEW_WORKOUT_TITLE: &str = "font-display text-[0.95rem] font-semibold leading-[1.25] \
     text-ink decoration-oxide/45 decoration-1 underline-offset-[0.18em] \
     hover:text-oxide hover:decoration-current focus-visible:text-oxide \
     focus-visible:decoration-current";
const PREVIEW_EXERCISE: &str = "font-meta text-[0.68rem] leading-[1.45] text-ink2";
const PREVIEW_LOG_LINK: &str = "mt-[0.75rem] inline-block font-meta text-[0.72rem] text-oxide \
     decoration-oxide/45 decoration-1 underline-offset-[0.18em] hover:decoration-current \
     focus-visible:decoration-current";

const HEATMAP_PREVIEW_JS: Asset = asset!("./heatmap-preview.js");

/// Page-only running presence by Eastern calendar day. This stays separate
/// from [`CalendarDay`], whose public API contract is lifting volume only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RunDay {
    pub(super) date: String,
    pub(super) count: usize,
}

/// Collapse independently stored runs into the small day/count seam the
/// heatmap needs. Invalid dates are dropped defensively; imported rows already
/// carry a schema-checked Eastern projection.
pub(super) fn run_days(activities: &[RunningActivity]) -> Vec<RunDay> {
    let mut counts = BTreeMap::<String, usize>::new();
    for activity in activities {
        let date = running::activity_date(activity);
        if date.parse::<Date>().is_err() {
            continue;
        }
        let count = counts.entry(date.to_string()).or_default();
        *count = count.saturating_add(1);
    }
    counts
        .into_iter()
        .map(|(date, count)| RunDay { date, count })
        .collect()
}

/// Composite training calendar. `link_query` carries the log page's active filters
/// (canonical, minus `from`/`to`/`page`) into the preview's day-log link.
/// Day activity bodies are not embedded here — they load through
/// [`day_preview_shard`] when `preview_day` is set.
/// Runs and interruptions remain overlays: neither changes lifting volume.
#[component]
pub(super) async fn calendar_heatmap(
    days: Vec<CalendarDay>,
    #[default(Vec::new())] runs: Vec<RunDay>,
    #[default(String::new())] link_query: String,
    #[default(false)] filtered: bool,
    #[default(Vec::new())] interruptions: Vec<Interruption>,
) -> Result {
    // Always run through today (Eastern), or a later training day if one exists.
    let through = eastern::eastern_date(Timestamp::now());
    let Some(calendar) = Calendar::from_days(&days, &runs, &interruptions, through) else {
        return view! {
            <section aria-labelledby="fitness-heatmap-title">
                <header
                    class="flex flex-wrap items-end justify-between gap-y-[0.8rem] gap-x-5"
                >
                    <div>
                        <p class=(META_LABEL)>"fitness archive"</p>
                        <h2
                            id="fitness-heatmap-title"
                            class="font-display text-2xl font-semibold"
                        >
                            "Training days"
                        </h2>
                    </div>
                </header>
                <p class=(class!(HEAT_NOTE, "mt-[0.8rem]"))>
                    if filtered {
                        "No training days match these filters."
                    } else {
                        "No training days are available yet."
                    }
                </p>
            </section>
        };
    };

    let ending = format_short(calendar.latest);
    let start = format_short(calendar.latest - 53.weeks());
    let counts = format!(
        "{} lift {} · {} {} on {} run {}",
        calendar.lift_days,
        plural(calendar.lift_days, "day", "days"),
        calendar.run_count,
        plural(calendar.run_count, "run", "runs"),
        calendar.run_days,
        plural(calendar.run_days, "day", "days"),
    );
    let subtitle = if filtered {
        format!("{start} - {ending} · matching view · {counts}")
    } else {
        format!("{start} - {ending} · {counts}")
    };
    let filter_copy = if filtered {
        " from sets matching the active filters"
    } else {
        ""
    };
    let navigation_label = format!(
        "Training by day for the 53 weeks ending {ending}. Oxide fill shows lifting volume points{filter_copy}; a patina corner marks one or more runs; emoji mark interruptions. {} training days open an activity preview.",
        calendar.activity_days,
    );
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
                    <p class=(META_LABEL)>"fitness archive"</p>
                    <h2
                        id="fitness-heatmap-title"
                        class="font-display text-2xl font-semibold"
                    >
                        "Training days"
                    </h2>
                    <p class=(class!(HEAT_NOTE, "mt-[0.3rem]"))>(subtitle.as_str())</p>
                </div>
                <div
                    class="inline-flex items-center gap-[0.22rem] font-meta text-[0.61rem] \
                         leading-none uppercase text-muted"
                    aria-label="Oxide fill is lifting volume from less to more. A patina corner marks a run."
                >
                    <span class="mr-[0.12rem]">"lift volume"</span>
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
                    <span class="ml-[0.45rem]">"run"</span>
                    <span
                        class=(class!(LEGEND_CELL, "relative overflow-hidden bg-card"))
                        aria-hidden="true"
                    >
                        <span class=(CELL_RUN_MARKER)></span>
                    </span>
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
/// from the browser and are validated before touching either archive.
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
        format!("/fitness/log?from={date}&to={date}#set-log")
    } else {
        format!("/fitness/log?{link_query}&from={date}&to={date}#set-log")
    };

    let (snapshot, run_log) = tokio::join!(
        app_context::<FitnessStore>(cx).snapshot(),
        running::load(app_context::<Data>(cx)),
    );
    let lifts_unavailable = snapshot.is_err();
    let runs_unavailable = !run_log.live;
    let snapshot = snapshot.ok();
    let summaries = snapshot
        .as_ref()
        .map(|snapshot| snapshot.workouts_on_date(&date))
        .unwrap_or_default();
    let marks = snapshot
        .as_ref()
        .map(|snapshot| interruptions::marks_covering_today(snapshot.interruptions(), &date))
        .unwrap_or_default();
    let interrupted_copy = (!marks.is_empty()).then(|| {
        format!(
            "Interrupted · {}",
            marks
                .iter()
                .map(|mark| format!("{} {}", mark.emoji, mark.note))
                .collect::<Vec<_>>()
                .join(" · ")
        )
    });
    let day_runs: Vec<&RunningActivity> = run_log
        .activities
        .iter()
        .filter(|activity| running::activity_date(activity) == date)
        .collect();
    let run_count = day_runs.len();

    let mut volume_points = 0_u32;
    let mut activities = Vec::with_capacity(summaries.len() + run_count);
    for summary in summaries {
        volume_points = volume_points.saturating_add(summary.volume_points);
        let involvement = snapshot
            .as_ref()
            .map(|snapshot| {
                muscles::involvement_for_exercises(
                    summary.exercises.iter().map(String::as_str),
                    snapshot.exercise_weight_map(),
                )
            })
            .unwrap_or_default();
        activities.push(ShardActivity::Workout(ShardWorkout {
            identity: summary.path.clone(),
            start_time: summary.start_time,
            title: summary.title,
            href: workout_url(&summary.path),
            duration: format_duration(summary.duration_seconds),
            set_count: summary.set_count,
            exercises: summary.exercises,
            involvement,
        }));
    }
    for activity in day_runs {
        activities.push(ShardActivity::Run(ShardRun {
            identity: activity.id.clone(),
            start_time: running::start_time_seconds(activity),
            title: activity.title.clone(),
            href: running::public_url(activity),
            distance: running::distance_label(activity),
            duration: running::duration_label(activity),
            pace: running::pace_label(activity),
        }));
    }
    sort_shard_activities(&mut activities);

    if activities.is_empty() {
        return view! {
            <span class="inline-popover-kicker">(format_long(parsed).as_str())</span>
            if let Some(copy) = &interrupted_copy {
                <p class=(PREVIEW_INTERRUPTION)>(copy.as_str())</p>
            }
            <p class=(HEAT_NOTE)>"No training activities are stored for this day."</p>
            if lifts_unavailable {
                <p class=(HEAT_NOTE)>"Lift details are unavailable right now."</p>
            }
            if runs_unavailable {
                <p class=(HEAT_NOTE)>"Run details are unavailable right now."</p>
            }
            <a class=(PREVIEW_LOG_LINK) href=(href.as_str())>"view day in log →"</a>
        };
    }

    let heading = format_long(parsed);
    let detail_label = format!(
        "{volume_points} lifting volume {} · {run_count} {}",
        plural(volume_points as usize, "point", "points"),
        plural(run_count, "run", "runs"),
    );

    view! {
        <span class="inline-popover-kicker">(heading.as_str())</span>
        <span class="inline-popover-detail">(detail_label.as_str())</span>
        if let Some(copy) = &interrupted_copy {
            <p class=(PREVIEW_INTERRUPTION)>(copy.as_str())</p>
        }
        if lifts_unavailable {
            <p class=(HEAT_NOTE)>"Lift details are unavailable right now."</p>
        }
        if runs_unavailable {
            <p class=(HEAT_NOTE)>"Run details are unavailable right now."</p>
        }
        <div class="space-y-[0.75rem]">
            for activity in activities.iter() {
                if let ShardActivity::Workout(workout) = activity {
                    workout_block(workout: workout)
                }
                if let ShardActivity::Run(run) = activity {
                    run_block(run: run)
                }
            }
        </div>
        <a class=(PREVIEW_LOG_LINK) href=(href.as_str())>"view day in log →"</a>
    }
}

#[component]
async fn run_block(run: &ShardRun) -> Result {
    let meta = format!("{} · {}", run.distance, run.duration);
    let title_label = format!("Open {} run", run.title);
    view! {
        <article class="border-t border-hairline pt-[0.65rem] first:border-t-0 first:pt-0">
            <header class="flex items-baseline justify-between gap-3">
                <a
                    class=(PREVIEW_WORKOUT_TITLE)
                    href=(run.href.as_str())
                    aria-label=(title_label.as_str())
                >
                    (run.title.as_str())
                </a>
                <span class="flex-none font-meta text-[0.62rem] leading-[1.4] text-muted">
                    (meta.as_str())
                </span>
            </header>
            <p class=(class!(PREVIEW_EXERCISE, "mt-[0.3rem] text-patina"))>
                "run · " (run.pace.as_str())
            </p>
        </article>
    }
}

enum ShardActivity {
    Workout(ShardWorkout),
    Run(ShardRun),
}

impl ShardActivity {
    fn start_time(&self) -> i64 {
        match self {
            Self::Workout(workout) => workout.start_time,
            Self::Run(run) => run.start_time,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Self::Workout(_) => 0,
            Self::Run(_) => 1,
        }
    }

    fn identity(&self) -> &str {
        match self {
            Self::Workout(workout) => &workout.identity,
            Self::Run(run) => &run.identity,
        }
    }
}

fn sort_shard_activities(activities: &mut [ShardActivity]) {
    activities.sort_by(|a, b| {
        b.start_time()
            .cmp(&a.start_time())
            .then_with(|| a.rank().cmp(&b.rank()))
            .then_with(|| a.identity().cmp(b.identity()))
    });
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
    identity: String,
    start_time: i64,
    title: String,
    href: String,
    duration: String,
    set_count: usize,
    exercises: Vec<String>,
    involvement: muscles::MuscleInvolvement,
}

struct ShardRun {
    identity: String,
    start_time: i64,
    title: String,
    href: String,
    distance: String,
    duration: String,
    pace: String,
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
                if let Some(emoji) = &cell.emoji {
                    <span class=(CELL_EMOJI) aria-hidden="true">(emoji.as_str())</span>
                }
                if cell.run_count > 0 {
                    <span class=(CELL_RUN_MARKER) aria-hidden="true"></span>
                }
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
    lift_days: usize,
    run_days: usize,
    run_count: usize,
    activity_days: usize,
    cells: Vec<HeatmapCell>,
    month_labels: Vec<MonthLabel>,
}

impl Calendar {
    /// 53 Sunday–Saturday weeks ending on the Saturday of the week that
    /// contains `through`, after raising `through` to any later logged day.
    /// That keeps recent empty days (and interruption chrome) visible even
    /// when the newest lift is older than today.
    fn from_days(
        days: &[CalendarDay],
        runs: &[RunDay],
        interruptions: &[Interruption],
        through: Date,
    ) -> Option<Self> {
        let mut points_by_day = BTreeMap::new();
        for day in days {
            let date: Date = day.date.parse().ok()?;
            let points = points_by_day.entry(date).or_insert(0_u32);
            *points = points.saturating_add(day.volume_points);
        }
        let mut runs_by_day = BTreeMap::new();
        for run in runs.iter().filter(|run| run.count > 0) {
            let date: Date = run.date.parse().ok()?;
            let count = runs_by_day.entry(date).or_insert(0_usize);
            *count = count.saturating_add(run.count);
        }
        if points_by_day.is_empty() && runs_by_day.is_empty() && interruptions.is_empty() {
            return None;
        }
        let latest = points_by_day
            .keys()
            .chain(runs_by_day.keys())
            .max()
            .copied()
            .map_or(through, |logged| logged.max(through));
        let end_offset =
            DAYS_PER_WEEK as i64 - 1 - i64::from(latest.weekday().to_sunday_zero_offset());
        let end = latest.checked_add(end_offset.days()).ok()?;
        let start = end.checked_add((-(CELL_COUNT as i64 - 1)).days()).ok()?;
        let mut cells = Vec::with_capacity(CELL_COUNT);
        let mut lift_days = 0;
        let mut run_days = 0;
        let mut run_count = 0_usize;
        let mut activity_days = 0;
        for offset in 0..CELL_COUNT {
            let date = start + (offset as i64).days();
            let date_key = date.to_string();
            let points = points_by_day.get(&date).copied().unwrap_or(0);
            let has_lift = points_by_day.contains_key(&date);
            let day_run_count = runs_by_day.get(&date).copied().unwrap_or(0);
            let marks =
                interruptions::marks_covering(interruptions, &date_key, &through.to_string());
            if has_lift {
                lift_days += 1;
            }
            if day_run_count > 0 {
                run_days += 1;
                run_count = run_count.saturating_add(day_run_count);
            }
            if has_lift || day_run_count > 0 {
                activity_days += 1;
            }
            cells.push(HeatmapCell::new(
                date,
                points,
                has_lift,
                day_run_count,
                &marks,
            ));
        }
        let month_labels = MonthLabel::from_cells(&cells);
        Some(Self {
            latest,
            lift_days,
            run_days,
            run_count,
            activity_days,
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
    emoji: Option<String>,
    run_count: usize,
}

impl HeatmapCell {
    fn new(
        date: Date,
        points: u32,
        has_lift: bool,
        run_count: usize,
        marks: &[interruptions::DayMark<'_>],
    ) -> Self {
        let intensity = intensity(points);
        let interrupted = !marks.is_empty();
        let border = if has_lift && points == 0 {
            CELL_BORDER_ZERO
        } else {
            CELL_BORDER
        };
        let date_label = format_long(date);
        let lift_label = if has_lift {
            format!(
                "{points} lifting volume {}",
                plural(points as usize, "point", "points")
            )
        } else {
            "no lifting volume".to_string()
        };
        let run_label =
            (run_count > 0).then(|| format!("{run_count} {}", plural(run_count, "run", "runs")));
        let notes_label = marks
            .iter()
            .map(|mark| format!("{} {}", mark.emoji, mark.note))
            .collect::<Vec<_>>()
            .join(" · ");
        let mut details = vec![lift_label];
        if let Some(run_label) = run_label {
            details.push(run_label);
        }
        if interrupted {
            details.push(format!("Interrupted · {notes_label}"));
        }
        let action = if has_lift || run_count > 0 {
            " Preview activities from this day."
        } else if interrupted {
            " Open this day's interruption note."
        } else {
            ""
        };
        let label = format!("{date_label}: {}.{action}", details.join(". "));
        Self {
            date,
            // Interrupted empty days open the same preview so the note is readable.
            date_key: (has_lift || run_count > 0 || interrupted).then(|| date.to_string()),
            border,
            label,
            style: heat_style(intensity),
            emoji: marks.first().map(|mark| mark.emoji.to_string()),
            run_count,
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

    fn run(id: &str, local_date: &str, utc_time: &str) -> RunningActivity {
        RunningActivity {
            id: id.to_string(),
            source: "garmin-connect".to_string(),
            source_activity_id: id.to_string(),
            source_url: None,
            title: format!("Run {id}"),
            activity_type: "running".to_string(),
            started_at_utc: utc_time.to_string(),
            started_at_local: format!("{local_date} 10:00:00"),
            eastern_offset_minutes: -240,
            duration_milliseconds: 1_800_000,
            moving_duration_milliseconds: Some(1_780_000),
            distance_millimeters: 5_000_000,
            ascent_millimeters: Some(30_000),
            imported_at: 1,
        }
    }

    #[test]
    fn grid_is_53_complete_sunday_to_saturday_weeks_anchored_to_latest_day() {
        let through = "2026-07-21".parse().unwrap();
        let calendar =
            Calendar::from_days(&[day("2026-07-21", 42)], &[], &[], through).expect("calendar");

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
            "Sunday, Jul 20, 2025: no lifting volume."
        );
        assert_eq!(
            calendar.cells[366].label,
            "Tuesday, Jul 21, 2026: 42 lifting volume points. Preview activities from this day."
        );
        assert_eq!(calendar.cells[366].date_key.as_deref(), Some("2026-07-21"));
        assert!(calendar.cells[367].date_key.is_none());
    }

    #[test]
    fn grid_extends_through_today_when_newest_lift_is_older() {
        let through = "2026-08-11".parse().unwrap();
        let rows = [Interruption {
            id: "a".into(),
            from_date: "2026-08-02".into(),
            to_date: Some("2026-08-09".into()),
            note: "cold".into(),
            emoji: "🤒".into(),
            updated_at: 0,
        }];
        let calendar =
            Calendar::from_days(&[day("2026-07-21", 42)], &[], &rows, through).expect("calendar");

        assert_eq!(calendar.latest.to_string(), "2026-08-11");
        let interrupted = calendar
            .cells
            .iter()
            .find(|cell| cell.date.to_string() == "2026-08-05")
            .expect("day inside interruption");
        assert!(interrupted.label.contains("Interrupted · 🤒 cold"));
        assert_eq!(interrupted.emoji.as_deref(), Some("🤒"));
        assert_eq!(interrupted.date_key.as_deref(), Some("2026-08-05"));
        // Future relative to through still wins if a workout lands there.
        let ahead = Calendar::from_days(
            &[day("2026-08-20", 10)],
            &[],
            &[],
            "2026-08-11".parse().unwrap(),
        )
        .expect("future calendar");
        assert_eq!(ahead.latest.to_string(), "2026-08-20");
    }

    #[test]
    fn interrupted_empty_days_are_linkable_and_labeled() {
        let rows = [Interruption {
            id: "a".into(),
            from_date: "2026-07-20".into(),
            to_date: Some("2026-07-20".into()),
            note: "cold".into(),
            emoji: "🤧".into(),
            updated_at: 0,
        }];
        let through = "2026-07-21".parse().unwrap();
        let calendar =
            Calendar::from_days(&[day("2026-07-21", 42)], &[], &rows, through).expect("calendar");
        let cell = calendar
            .cells
            .iter()
            .find(|cell| cell.date.to_string() == "2026-07-20")
            .expect("interrupted day");
        assert_eq!(cell.date_key.as_deref(), Some("2026-07-20"));
        assert!(cell.label.contains("Interrupted · 🤧 cold"));
        assert_eq!(cell.emoji.as_deref(), Some("🤧"));
        assert_eq!(cell.border, CELL_BORDER);
    }

    #[test]
    fn open_interruptions_cover_through_today_only() {
        let through = "2026-08-11".parse().unwrap();
        let rows = [Interruption {
            id: "a".into(),
            from_date: "2026-08-02".into(),
            to_date: None,
            note: "ongoing".into(),
            emoji: "😴".into(),
            updated_at: 0,
        }];
        let calendar = Calendar::from_days(&[], &[], &rows, through).expect("calendar");
        let covered = calendar
            .cells
            .iter()
            .find(|cell| cell.date.to_string() == "2026-08-11")
            .expect("today");
        assert_eq!(covered.emoji.as_deref(), Some("😴"));
        let future = calendar
            .cells
            .iter()
            .find(|cell| cell.date.to_string() == "2026-08-12")
            .expect("day after today still in week");
        assert!(future.emoji.is_none());
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
        let through = "2024-02-29".parse().unwrap();
        let calendar = Calendar::from_days(
            &[day("2024-02-29", 20), day("2024-02-29", 25)],
            &[],
            &[],
            through,
        )
        .expect("calendar");
        let leap_day = calendar
            .cells
            .iter()
            .find(|cell| cell.date_key.as_deref() == Some("2024-02-29"))
            .expect("leap day cell");
        assert!(leap_day.label.contains("45 lifting volume points"));
        assert_eq!(leap_day.style, heat_style(3));
    }

    #[test]
    fn empty_archive_still_renders_through_today_when_interruptions_exist() {
        let through = "2026-08-11".parse().unwrap();
        assert!(Calendar::from_days(&[], &[], &[], through).is_none());
        let rows = [Interruption {
            id: "a".into(),
            from_date: "2026-08-02".into(),
            to_date: Some("2026-08-09".into()),
            note: "cold".into(),
            emoji: "🤒".into(),
            updated_at: 0,
        }];
        let calendar = Calendar::from_days(&[], &[], &rows, through).expect("calendar");
        assert_eq!(calendar.latest.to_string(), "2026-08-11");
        assert_eq!(calendar.lift_days, 0);
        assert_eq!(calendar.run_days, 0);
        assert_eq!(calendar.run_count, 0);
        assert_eq!(calendar.activity_days, 0);
        assert_eq!(calendar.cells.len(), 371);
    }

    #[test]
    fn run_days_group_by_valid_eastern_date_in_order() {
        let activities = [
            run("later", "2026-08-20", "2026-08-20 14:00:00"),
            run("first", "2026-08-19", "2026-08-19 14:00:00"),
            run("second", "2026-08-20", "2026-08-20 16:00:00"),
            run("invalid", "not-a-date", "2026-08-20 18:00:00"),
        ];

        assert_eq!(
            run_days(&activities),
            vec![
                RunDay {
                    date: "2026-08-19".to_string(),
                    count: 1,
                },
                RunDay {
                    date: "2026-08-20".to_string(),
                    count: 2,
                },
            ]
        );
    }

    #[test]
    fn run_only_day_is_clickable_without_lift_intensity() {
        let through = "2026-08-11".parse().unwrap();
        let runs = [RunDay {
            date: "2026-08-20".to_string(),
            count: 2,
        }];
        let calendar = Calendar::from_days(&[], &runs, &[], through).expect("calendar");
        let run_day = calendar
            .cells
            .iter()
            .find(|cell| cell.date.to_string() == "2026-08-20")
            .expect("run day");

        assert_eq!(calendar.latest.to_string(), "2026-08-20");
        assert_eq!(calendar.lift_days, 0);
        assert_eq!(calendar.run_days, 1);
        assert_eq!(calendar.run_count, 2);
        assert_eq!(calendar.activity_days, 1);
        assert_eq!(run_day.date_key.as_deref(), Some("2026-08-20"));
        assert_eq!(run_day.run_count, 2);
        assert_eq!(run_day.style, heat_style(0));
        assert_eq!(run_day.border, CELL_BORDER);
        assert!(run_day.label.contains("no lifting volume. 2 runs"));
    }

    #[test]
    fn lift_run_and_interruption_share_a_cell_without_changing_lift_heat() {
        let through = "2026-08-20".parse().unwrap();
        let runs = [
            RunDay {
                date: "2026-08-20".to_string(),
                count: 1,
            },
            RunDay {
                date: "2026-08-20".to_string(),
                count: 2,
            },
        ];
        let interruptions = [Interruption {
            id: "a".into(),
            from_date: "2026-08-20".into(),
            to_date: Some("2026-08-20".into()),
            note: "ankle".into(),
            emoji: "🩹".into(),
            updated_at: 0,
        }];
        let calendar =
            Calendar::from_days(&[day("2026-08-20", 24)], &runs, &interruptions, through)
                .expect("calendar");
        let cell = calendar
            .cells
            .iter()
            .find(|cell| cell.date.to_string() == "2026-08-20")
            .expect("combined day");

        assert_eq!(calendar.lift_days, 1);
        assert_eq!(calendar.run_days, 1);
        assert_eq!(calendar.run_count, 3);
        assert_eq!(calendar.activity_days, 1);
        assert_eq!(cell.style, heat_style(1));
        assert_eq!(cell.run_count, 3);
        assert_eq!(cell.emoji.as_deref(), Some("🩹"));
        assert!(cell.label.contains("24 lifting volume points"));
        assert!(cell.label.contains("3 runs"));
        assert!(cell.label.contains("Interrupted · 🩹 ankle"));
    }

    #[test]
    fn preview_activities_sort_by_exact_utc_then_kind_and_identity() {
        let mut activities = vec![
            ShardActivity::Run(ShardRun {
                identity: "run-latest".into(),
                start_time: 200,
                title: String::new(),
                href: String::new(),
                distance: String::new(),
                duration: String::new(),
                pace: String::new(),
            }),
            ShardActivity::Run(ShardRun {
                identity: "run-tied".into(),
                start_time: 100,
                title: String::new(),
                href: String::new(),
                distance: String::new(),
                duration: String::new(),
                pace: String::new(),
            }),
            ShardActivity::Workout(ShardWorkout {
                identity: "lift-z".into(),
                start_time: 100,
                title: String::new(),
                href: String::new(),
                duration: String::new(),
                set_count: 0,
                exercises: Vec::new(),
                involvement: muscles::MuscleInvolvement::default(),
            }),
            ShardActivity::Workout(ShardWorkout {
                identity: "lift-a".into(),
                start_time: 100,
                title: String::new(),
                href: String::new(),
                duration: String::new(),
                set_count: 0,
                exercises: Vec::new(),
                involvement: muscles::MuscleInvolvement::default(),
            }),
        ];

        sort_shard_activities(&mut activities);

        assert_eq!(
            activities
                .iter()
                .map(ShardActivity::identity)
                .collect::<Vec<_>>(),
            vec!["run-latest", "lift-a", "lift-z", "run-tied"]
        );
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
