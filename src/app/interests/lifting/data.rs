//! Server-side lifting reader for `/fitness` — in-process over the fitness
//! snapshot (`benjisponge::fitness`), no HTTP hop.
//!
//! The wire types are the lib's own API envelopes, so pages and the
//! public JSON endpoints can never drift. `LoadError` keeps its old
//! shape: `Rejected` messages are the filter validator's exact 400
//! strings and stay reader-visible; everything else renders generically.

use std::fmt;

use super::archive::eastern;
use super::archive::filters::{Filters, parse_filters};
use super::archive::snapshot::FilteredWorkout;
use super::archive::store::FitnessStore;
use super::training_focus::TrainingFocus;
use benjisponge::data::running_models::RunningActivity;

pub use super::archive::api::{Calendar, CalendarDay, Facets, Record, Set, Workout, WorkoutDetail};
pub use benjisponge::data::fitness_models::Interruption;

/// A running activity admitted by the universal fitness-log filters. The
/// stored run stays a sibling model; this wrapper adds only page-local sort
/// projections and never enters `FitnessStore` or a public JSON envelope.
#[derive(Clone, Debug)]
pub(in crate::app::interests::lifting) struct FilteredRun {
    pub activity: RunningActivity,
    pub date: String,
    pub start_time: i64,
}

/// One primary row in the composite fitness log.
#[derive(Clone, Debug)]
pub(in crate::app::interests::lifting) enum LogActivity {
    Lift(FilteredWorkout),
    Run(FilteredRun),
}

impl LogActivity {
    pub(in crate::app::interests::lifting) fn date(&self) -> &str {
        match self {
            Self::Lift(lift) => &lift.date,
            Self::Run(run) => &run.date,
        }
    }

    fn start_time(&self) -> i64 {
        match self {
            Self::Lift(lift) => lift.start_time,
            Self::Run(run) => run.start_time,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Self::Lift(_) => 0,
            Self::Run(_) => 1,
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Lift(lift) => &lift.workout.id,
            Self::Run(run) => &run.activity.id,
        }
    }
}

/// Page-only composition for `/fitness/log`. `activities` contains only the
/// primary rows on this page; closed interruptions consume no slots and are
/// assigned to the page containing the last same-date primary row.
#[derive(Clone, Debug)]
pub(in crate::app::interests::lifting) struct FitnessLogPage {
    pub page: usize,
    pub per_page: usize,
    pub total_sets: u64,
    pub total_lifts: u64,
    pub total_runs: u64,
    /// Every run admitted by the current universal filters, before activity
    /// pagination. The page uses this only to build the matching heatmap.
    pub matching_runs: Vec<RunningActivity>,
    pub activities: Vec<LogActivity>,
    pub interruptions: Vec<Interruption>,
}

impl FitnessLogPage {
    pub(in crate::app::interests::lifting) fn total_activities(&self) -> usize {
        (self.total_lifts + self.total_runs) as usize
    }
}

/// A rejected filter is safe to show to the reader. Snapshot failures are
/// logged by the page but deliberately rendered generically.
#[derive(Debug)]
pub enum LoadError {
    Rejected(String),
    NotFound(String),
    Unavailable(String),
}

impl LoadError {
    pub fn rejected_message(&self) -> Option<&str> {
        match self {
            Self::Rejected(message) => Some(message),
            Self::NotFound(_) | Self::Unavailable(_) => None,
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(message) => write!(formatter, "fitness filter rejected: {message}"),
            Self::NotFound(message) => write!(formatter, "fitness resource not found: {message}"),
            Self::Unavailable(message) => {
                write!(formatter, "fitness archive unavailable: {message}")
            }
        }
    }
}

/// The full-log page's reads. Lift facets, matches, calendar, and
/// interruptions all come from one snapshot. Runs are loaded independently
/// by the page and supplied only when no lift-only filter is active.
pub(in crate::app::interests::lifting) async fn load(
    store: &FitnessStore,
    filters: &[(String, String)],
    runs: &[RunningActivity],
) -> (
    Result<Facets, LoadError>,
    Result<FitnessLogPage, LoadError>,
    Result<Calendar, LoadError>,
    Result<Vec<Interruption>, LoadError>,
) {
    let snapshot = match store.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let message = error.to_string();
            return (
                Err(LoadError::Unavailable(message.clone())),
                Err(LoadError::Unavailable(message.clone())),
                Err(LoadError::Unavailable(message.clone())),
                Err(LoadError::Unavailable(message)),
            );
        }
    };
    let (page, calendar) = match parse_filters(filters) {
        Ok(parsed) => {
            let lifts = snapshot.filtered_workouts(&parsed);
            (
                Ok(compose_log_page(
                    lifts,
                    runs,
                    snapshot.interruptions(),
                    &parsed,
                )),
                Ok(snapshot.calendar_filtered(&parsed)),
            )
        }
        Err(message) => (
            Err(LoadError::Rejected(message.clone())),
            Err(LoadError::Rejected(message)),
        ),
    };
    (
        Ok(snapshot.facets()),
        page,
        calendar,
        Ok(snapshot.interruptions().to_vec()),
    )
}

fn compose_log_page(
    lifts: Vec<FilteredWorkout>,
    runs: &[RunningActivity],
    interruptions: &[Interruption],
    filters: &Filters,
) -> FitnessLogPage {
    let total_sets = lifts
        .iter()
        .map(|lift| lift.workout.sets.len() as u64)
        .sum();
    let total_lifts = lifts.len() as u64;
    let mut activities: Vec<LogActivity> = lifts.into_iter().map(LogActivity::Lift).collect();
    if filters.admits_runs() {
        activities.extend(
            runs.iter()
                .filter_map(|activity| filtered_run(activity, filters))
                .map(LogActivity::Run),
        );
    }
    activities.sort_by(|left, right| {
        right
            .start_time()
            .cmp(&left.start_time())
            .then_with(|| left.rank().cmp(&right.rank()))
            .then_with(|| right.id().cmp(left.id()))
    });

    let total_runs = activities
        .iter()
        .filter(|activity| matches!(activity, LogActivity::Run(_)))
        .count() as u64;
    let matching_runs = activities
        .iter()
        .filter_map(|activity| match activity {
            LogActivity::Run(run) => Some(run.activity.clone()),
            LogActivity::Lift(_) => None,
        })
        .collect();
    let page_interruptions = interruptions_for_page(&activities, interruptions, filters);
    let offset = (filters.page - 1) * filters.per_page;
    let activities = activities
        .into_iter()
        .skip(offset)
        .take(filters.per_page)
        .collect();
    FitnessLogPage {
        page: filters.page,
        per_page: filters.per_page,
        total_sets,
        total_lifts,
        total_runs,
        matching_runs,
        activities,
        interruptions: page_interruptions,
    }
}

fn filtered_run(activity: &RunningActivity, filters: &Filters) -> Option<FilteredRun> {
    let date = activity.started_at_local.get(..10)?;
    if filters.from.as_deref().is_some_and(|from| date < from)
        || filters.to.as_deref().is_some_and(|to| date > to)
    {
        return None;
    }
    let hour = activity.started_at_local.get(11..13)?.parse::<u8>().ok()?;
    if filters
        .time_of_day
        .is_some_and(|band| !band.contains_hour(hour))
    {
        return None;
    }
    if filters
        .weekday
        .is_some_and(|wanted| weekday_sunday_zero(date) != Some(wanted))
    {
        return None;
    }
    let start_time = eastern::utc_timestamp(&activity.started_at_utc)
        .ok()?
        .as_second();
    Some(FilteredRun {
        activity: activity.clone(),
        date: date.to_string(),
        start_time,
    })
}

fn weekday_sunday_zero(date: &str) -> Option<u8> {
    let year = date.get(..4)?.parse().ok()?;
    let month = date.get(5..7)?.parse().ok()?;
    let day = date.get(8..10)?.parse().ok()?;
    jiff::civil::Date::new(year, month, day)
        .ok()
        .map(|date| date.weekday().to_sunday_zero_offset() as u8)
}

fn interruptions_for_page(
    activities: &[LogActivity],
    interruptions: &[Interruption],
    filters: &Filters,
) -> Vec<Interruption> {
    interruptions
        .iter()
        .filter(|row| {
            let Some(to_date) = row.to_date.as_deref() else {
                return false;
            };
            if filters.from.as_deref().is_some_and(|from| to_date < from)
                || filters.to.as_deref().is_some_and(|to| to_date > to)
            {
                return false;
            }
            let preceding = activities
                .iter()
                .filter(|activity| activity.date() >= to_date)
                .count();
            let assigned_page = if preceding == 0 {
                1
            } else {
                (preceding - 1) / filters.per_page.max(1) + 1
            };
            assigned_page == filters.page
        })
        .cloned()
        .collect()
}

/// The landing view: archive-wide daily totals plus the newest workout.
pub async fn load_home(
    store: &FitnessStore,
) -> (
    Result<Calendar, LoadError>,
    Result<WorkoutDetail, LoadError>,
    Result<TrainingFocus, LoadError>,
    Result<Vec<Interruption>, LoadError>,
) {
    match store.snapshot().await {
        Ok(snapshot) => {
            let today = eastern::eastern_date(jiff::Timestamp::now());
            (
                Ok(snapshot.calendar()),
                Ok(snapshot.latest()),
                Ok(snapshot.training_focus(today)),
                Ok(snapshot.interruptions().to_vec()),
            )
        }
        Err(error) => {
            let message = error.to_string();
            (
                Err(LoadError::Unavailable(message.clone())),
                Err(LoadError::Unavailable(message.clone())),
                Err(LoadError::Unavailable(message.clone())),
                Err(LoadError::Unavailable(message)),
            )
        }
    }
}

/// Weighted muscle credit for the exercises one workout used, keyed by
/// canonical exercise name. Snapshot-derived and page-only — deliberately
/// not part of any public JSON envelope.
pub type ExerciseWeights = std::collections::HashMap<String, Vec<(&'static str, u32)>>;

/// Resolve a canonical public path. Rejections mirror the API's 404s.
/// The weights come from the same snapshot as the workout, so the muscle
/// summary always describes exactly the sets on the page.
pub async fn load_workout_by_path(
    store: &FitnessStore,
    path: &str,
) -> Result<(WorkoutDetail, ExerciseWeights), LoadError> {
    let Some(instant) = eastern::parse_public_path(path) else {
        return Err(LoadError::NotFound("not found".to_string()));
    };
    let snapshot = store
        .snapshot()
        .await
        .map_err(|error| LoadError::Unavailable(error.to_string()))?;
    let detail = snapshot
        .by_path(&instant)
        .ok_or_else(|| LoadError::NotFound("not found".to_string()))?;
    let mut weights = ExerciseWeights::new();
    if let Some(workout) = &detail.workout {
        let map = snapshot.exercise_weight_map();
        for set in &workout.sets {
            if !weights.contains_key(&set.exercise_name)
                && let Some(pairs) = map.get(&set.exercise_name)
            {
                weights.insert(set.exercise_name.clone(), pairs.clone());
            }
        }
    }
    Ok((detail, weights))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lift(id: &str, utc: &str, local: &str, set_count: usize) -> FilteredWorkout {
        let sets = (0..set_count)
            .map(|index| Set {
                id: format!("{id}:s{index}"),
                ordinal: index as u32 + 1,
                exercise_name: "Squat".into(),
                raw_exercise_name: "Squat".into(),
                exercise_note: None,
                superset_id: None,
                weight_milli: Some(100_000),
                weight_unit: "lbs".into(),
                reps: Some(5),
                effort_hundredths: None,
                distance_milli: None,
                set_time_seconds: None,
                set_type: "NORMAL_SET".into(),
                records: Vec::new(),
            })
            .collect();
        FilteredWorkout {
            workout: Workout {
                id: id.into(),
                path: id.into(),
                title: "Lift".into(),
                raw_title: "Lift".into(),
                started_at_local: local.into(),
                ended_at_local: local.into(),
                eastern_offset_minutes: -240,
                end_eastern_offset_minutes: -240,
                duration_seconds: 3_600,
                duration_suspicious: false,
                notes: None,
                description: None,
                sets,
            },
            date: local[..10].into(),
            start_time: eastern::utc_timestamp(utc).unwrap().as_second(),
        }
    }

    fn run(id: &str, utc: &str, local: &str) -> RunningActivity {
        RunningActivity {
            id: id.into(),
            source: "garmin-connect".into(),
            source_activity_id: id.into(),
            source_url: None,
            title: "Morning Run".into(),
            activity_type: "running".into(),
            started_at_utc: utc.into(),
            started_at_local: local.into(),
            eastern_offset_minutes: -240,
            duration_milliseconds: 1_800_000,
            moving_duration_milliseconds: None,
            distance_millimeters: 5_000_000,
            ascent_millimeters: None,
            imported_at: 0,
        }
    }

    #[test]
    fn parses_the_public_api_shape_and_preserves_nulls() {
        let page: super::super::archive::api::SetPage = serde_json::from_str(
            r#"{
              "version": 4, "page": 1, "per_page": 10,
              "total_sets": 1, "total_workouts": 1,
              "workouts": [{
                "id": "w1", "path": "2026-07-21T17-03-00-04-00",
                "title": "Leg day", "raw_title": "Leg day",
                "started_at_local": "2026-07-21 17:03:00",
                "ended_at_local": "2026-07-21 18:03:00",
                "eastern_offset_minutes": -240,
                "end_eastern_offset_minutes": -240,
                "duration_seconds": 3600, "duration_suspicious": false,
                "notes": null, "description": "hard",
                "sets": [{
                  "id": "s1", "ordinal": 1, "exercise_name": "Squat",
                  "raw_exercise_name": "Squat", "exercise_note": null,
                  "superset_id": null, "weight_milli": 102500, "weight_unit": "lbs", "reps": 5,
                  "effort_hundredths": null, "distance_milli": null,
                  "set_time_seconds": null, "set_type": "NORMAL_SET",
                  "records": [{"level": "gold", "kind": "volume"}]
                }]
              }]
            }"#,
        )
        .unwrap();

        assert_eq!(page.per_page, 10);
        assert_eq!(page.workouts[0].path, "2026-07-21T17-03-00-04-00");
        assert_eq!(page.workouts[0].eastern_offset_minutes, -240);
        assert_eq!(page.workouts[0].sets[0].weight_milli, Some(102_500));
        assert_eq!(page.workouts[0].sets[0].weight_unit, "lbs");
        assert_eq!(page.workouts[0].sets[0].effort_hundredths, None);
        assert_eq!(page.workouts[0].sets[0].records[0].kind, "volume");
    }

    #[test]
    fn parses_calendar_and_linkable_workout_envelopes() {
        let calendar: Calendar = serde_json::from_str(
            r#"{"version":4,"days":[{"date":"2026-07-21","volume_points":42}]}"#,
        )
        .unwrap();
        assert_eq!(calendar.days[0].date, "2026-07-21");
        assert_eq!(calendar.days[0].volume_points, 42);

        let detail: WorkoutDetail = serde_json::from_str(
            r#"{
              "version":4,
              "workout":null,
              "newer_workout_path":null,
              "older_workout_path":"2026-07-18T16-19-36-04-00"
            }"#,
        )
        .unwrap();
        assert!(detail.workout.is_none());
        assert_eq!(
            detail.older_workout_path.as_deref(),
            Some("2026-07-18T16-19-36-04-00")
        );
    }

    #[test]
    fn composite_page_sorts_exact_utc_and_counts_primary_rows() {
        let filters = Filters {
            page: 1,
            per_page: 2,
            ..Filters::default()
        };
        let page = compose_log_page(
            vec![
                lift("lift-new", "2026-08-21 15:00:00", "2026-08-21 11:00:00", 2),
                lift("lift-old", "2026-08-21 13:00:00", "2026-08-21 09:00:00", 1),
            ],
            &[
                run("run-middle", "2026-08-21 14:00:00", "2026-08-21 10:00:00"),
                run("run-old", "2026-08-20 14:00:00", "2026-08-20 10:00:00"),
            ],
            &[],
            &filters,
        );

        assert_eq!(page.total_sets, 3);
        assert_eq!(page.total_lifts, 2);
        assert_eq!(page.total_runs, 2);
        assert_eq!(page.total_activities(), 4);
        assert!(matches!(
            &page.activities[..],
            [LogActivity::Lift(lift), LogActivity::Run(run)]
                if lift.workout.id == "lift-new" && run.activity.id == "run-middle"
        ));
    }

    #[test]
    fn universal_filters_apply_to_runs_and_lift_filters_exclude_them() {
        let run = run(
            "friday-morning",
            "2026-08-21 11:00:00",
            "2026-08-21 07:00:00",
        );
        let universal = parse_filters(&[
            ("from".into(), "2026-08-21".into()),
            ("to".into(), "2026-08-21".into()),
            ("time_of_day".into(), "morning".into()),
            ("weekday".into(), "fri".into()),
            ("per_page".into(), "10".into()),
        ])
        .unwrap();
        assert_eq!(
            compose_log_page(Vec::new(), std::slice::from_ref(&run), &[], &universal).total_runs,
            1
        );

        let lift_only = parse_filters(&[
            ("q".into(), "morning".into()),
            ("per_page".into(), "10".into()),
        ])
        .unwrap();
        assert_eq!(
            compose_log_page(Vec::new(), &[run], &[], &lift_only).total_runs,
            0
        );
    }

    #[test]
    fn interruptions_use_no_slots_and_follow_the_last_same_date_primary() {
        let rows = [Interruption {
            id: "cold".into(),
            from_date: "2026-08-20".into(),
            to_date: Some("2026-08-21".into()),
            note: "cold".into(),
            emoji: "🤒".into(),
            updated_at: 0,
        }];
        let lifts = vec![
            lift("lift-a", "2026-08-21 15:00:00", "2026-08-21 11:00:00", 1),
            lift("lift-b", "2026-08-21 13:00:00", "2026-08-21 09:00:00", 1),
        ];
        let runs = [
            run("run-a", "2026-08-21 14:00:00", "2026-08-21 10:00:00"),
            run("run-b", "2026-08-20 14:00:00", "2026-08-20 10:00:00"),
        ];
        let first = compose_log_page(
            lifts.clone(),
            &runs,
            &rows,
            &Filters {
                page: 1,
                per_page: 2,
                ..Filters::default()
            },
        );
        assert_eq!(first.activities.len(), 2);
        assert!(first.interruptions.is_empty());

        let second = compose_log_page(
            lifts,
            &runs,
            &rows,
            &Filters {
                page: 2,
                per_page: 2,
                ..Filters::default()
            },
        );
        assert_eq!(second.activities.len(), 2, "interruption consumes no slot");
        assert_eq!(second.interruptions[0].id, "cold");
        assert_eq!(second.activities[0].date(), "2026-08-21");
        assert_eq!(second.activities[1].date(), "2026-08-20");
    }
}
