//! View models for workouts, set markers, records, and archive pagination.

use super::{
    data as fitness,
    filters::{Filters, SET_TYPES, lookup},
    format::{
        format_duration, format_scaled, format_signed_scaled, workout_datetime, workout_timing,
    },
};
use crate::util::urlencode;

pub(super) struct WorkoutCard<'a> {
    pub(super) href: String,
    pub(super) title: &'a str,
    pub(super) datetime: String,
    pub(super) date: String,
    pub(super) time_range: String,
    pub(super) duration: String,
    pub(super) duration_suspicious: bool,
    pub(super) description: Option<&'a str>,
    pub(super) notes: Option<&'a str>,
    pub(super) set_count: usize,
    pub(super) blocks: Vec<ExerciseBlock<'a>>,
}

impl<'a> From<&'a fitness::Workout> for WorkoutCard<'a> {
    fn from(workout: &'a fitness::Workout) -> Self {
        let timing = workout_timing(
            &workout.started_at_local,
            &workout.ended_at_local,
            workout.eastern_offset_minutes,
            workout.end_eastern_offset_minutes,
        );
        Self {
            href: workout_url(&workout.path),
            title: &workout.title,
            datetime: workout_datetime(&workout.started_at_local, workout.eastern_offset_minutes),
            date: timing.date,
            time_range: timing.range,
            duration: format_duration(workout.duration_seconds),
            duration_suspicious: workout.duration_suspicious,
            description: workout.description.as_deref(),
            notes: workout.notes.as_deref(),
            set_count: workout.sets.len(),
            blocks: exercise_blocks(&workout.sets, &workout.path),
        }
    }
}

pub(super) fn workout_url(path: &str) -> String {
    format!("/fitness/lift/{}", urlencode(path))
}

pub(super) struct ExerciseGroup<'a> {
    pub(super) name: &'a str,
    pub(super) superset_id: Option<u64>,
    pub(super) volume_points: u32,
    pub(super) rows: Vec<SetRow<'a>>,
}

pub(super) struct ExerciseBlock<'a> {
    pub(super) superset_id: Option<u64>,
    pub(super) groups: Vec<ExerciseGroup<'a>>,
}

pub(super) struct SetRow<'a> {
    pub(super) set: &'a fitness::Set,
    pub(super) effort_popover_id: String,
    pub(super) prescription: String,
    pub(super) details: String,
    pub(super) record: Option<String>,
    pub(super) note: Option<&'a str>,
}

pub(super) const RECORD_PR: &str = "inline-flex items-center min-h-[1.3rem] px-[0.36rem] \
     py-[0.18rem] border border-current rounded-[0.2rem] font-meta text-[0.56rem] \
     leading-none uppercase text-brass bg-brass/7";

/// Collapse a set's records into the one tag the page and share text show:
/// genuine all-time records only — the wire still carries the runner-up
/// podium places, but they are not presentation. A set that led every
/// category is plainly "PR"; otherwise the categories are named, except
/// that an estimated-1RM record subsumes the max-load record it almost
/// always accompanies (the load is already on the line).
pub(super) fn record_label(records: &[fitness::Record]) -> Option<String> {
    let gold: Vec<&str> = records
        .iter()
        .filter(|record| record.level == "gold")
        .map(|record| record.kind.as_str())
        .collect();
    if gold.is_empty() {
        return None;
    }
    if ["1rm", "max-weight", "volume", "reps"]
        .iter()
        .all(|kind| gold.contains(kind))
    {
        return Some("PR".to_string());
    }
    let subsumes_max_load = gold.contains(&"1rm");
    let kinds: Vec<&str> = gold
        .iter()
        .copied()
        .filter(|kind| !(subsumes_max_load && *kind == "max-weight"))
        .map(|kind| match kind {
            "1rm" => "1RM",
            "max-weight" => "max load",
            other => other,
        })
        .collect();
    Some(format!("PR: {}", kinds.join(" · ")))
}

pub(super) struct Pager {
    pub(super) newer: Option<String>,
    pub(super) older: Option<String>,
    pub(super) current: usize,
    pub(super) parts: Vec<Option<usize>>,
}

fn exercise_blocks<'a>(sets: &'a [fitness::Set], workout_path: &str) -> Vec<ExerciseBlock<'a>> {
    let mut groups = Vec::new();
    let mut start = 0;
    while start < sets.len() {
        let name = sets[start].exercise_name.as_str();
        let superset_id = sets[start].superset_id;
        let mut end = start + 1;
        while end < sets.len()
            && sets[end].exercise_name == name
            && sets[end].superset_id == superset_id
        {
            end += 1;
        }
        let slice = &sets[start..end];
        groups.push(ExerciseGroup {
            name,
            superset_id,
            volume_points: slice.iter().map(set_volume_points).sum(),
            rows: slice
                .iter()
                .map(|set| SetRow {
                    set,
                    effort_popover_id: effort_popover_id(workout_path, set.ordinal),
                    prescription: prescription(set),
                    details: set_details(set),
                    record: record_label(&set.records),
                    note: set.exercise_note.as_deref(),
                })
                .collect(),
        });
        start = end;
    }

    let mut blocks: Vec<ExerciseBlock<'a>> = Vec::new();
    for group in groups {
        if group.superset_id.is_some()
            && let Some(block) = blocks.last_mut()
            && block.superset_id == group.superset_id
        {
            block.groups.push(group);
        } else {
            blocks.push(ExerciseBlock {
                superset_id: group.superset_id,
                groups: vec![group],
            });
        }
    }
    blocks
}

/// Encodes the permanent workout path into a CSS custom-ident-safe ID. Set
/// ordinals are unique within a workout, so this stays unique across archive cards.
fn effort_popover_id(workout_path: &str, ordinal: u32) -> String {
    let path_hex: String = workout_path
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("lifting-effort-{path_hex}-{ordinal}")
}

fn set_volume_points(set: &fitness::Set) -> u32 {
    super::archive::scoring::set_volume_points(&set.set_type, set.effort_hundredths)
}

fn prescription(set: &fitness::Set) -> String {
    match (set.weight_milli, set.reps) {
        (Some(load), Some(reps)) => format!(
            "{} {} × {reps}",
            format_signed_scaled(load, 1_000),
            set.weight_unit
        ),
        (Some(load), None) => {
            format!("{} {}", format_signed_scaled(load, 1_000), set.weight_unit)
        }
        (None, Some(reps)) => format!("{reps} reps"),
        (None, None) => {
            if let Some(distance) = set.distance_milli {
                format!("distance {}", format_scaled(distance, 1_000))
            } else if let Some(seconds) = set.set_time_seconds {
                format_duration(seconds)
            } else {
                "not recorded".to_string()
            }
        }
    }
}

fn set_details(set: &fitness::Set) -> String {
    let mut details = Vec::new();
    if !matches!(
        set.set_type.as_str(),
        "NORMAL_SET" | "WARMUP_SET" | "FAILURE_SET" | ""
    ) {
        details.push(set_type_label(&set.set_type));
    }
    let load_or_reps_is_primary = set.weight_milli.is_some() || set.reps.is_some();
    if let Some(seconds) = set.set_time_seconds
        && (load_or_reps_is_primary || set.distance_milli.is_some())
    {
        details.push(format_duration(seconds));
    }
    if let Some(distance) = set.distance_milli
        && load_or_reps_is_primary
    {
        details.push(format!("distance {}", format_scaled(distance, 1_000)));
    }
    if set.reps.is_none() && set.distance_milli.is_none() && set.set_time_seconds.is_none() {
        details.push("incomplete".to_string());
    }
    details.join(" · ")
}

fn set_type_label(value: &str) -> String {
    lookup(SET_TYPES, value)
        .map(str::to_string)
        .unwrap_or_else(|| value.to_ascii_lowercase().replace('_', " "))
}

pub(super) fn make_pager(page: &fitness::FitnessLogPage, filters: &Filters) -> Option<Pager> {
    let pages = total_pages(page);
    if pages <= 1 {
        return None;
    }
    let current = page.page.clamp(1, pages);
    Some(Pager {
        newer: (current > 1).then(|| filters.page_url(current - 1)),
        older: (current < pages).then(|| filters.page_url(current + 1)),
        current,
        parts: page_window(current, pages),
    })
}

pub(super) fn total_pages(page: &fitness::FitnessLogPage) -> usize {
    page.total_activities()
        .div_ceil(page.per_page.max(1))
        .max(1)
}

fn page_window(current: usize, total: usize) -> Vec<Option<usize>> {
    let mut pages = vec![1, total];
    pages.extend(current.saturating_sub(2).max(1)..=(current + 2).min(total));
    pages.sort_unstable();
    pages.dedup();
    let mut output = Vec::new();
    for page in pages {
        if output
            .last()
            .and_then(|value| *value)
            .is_some_and(|previous| page > previous + 1)
        {
            output.push(None);
        }
        output.push(Some(page));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set() -> fitness::Set {
        fitness::Set {
            id: "fitness:2026-07-21T21:03:00:0001".to_string(),
            ordinal: 1,
            exercise_name: "Plank".to_string(),
            raw_exercise_name: "Plank".to_string(),
            exercise_note: None,
            superset_id: None,
            weight_milli: None,
            weight_unit: "lbs".to_string(),
            reps: None,
            effort_hundredths: None,
            distance_milli: None,
            set_time_seconds: None,
            set_type: "NORMAL_SET".to_string(),
            records: Vec::new(),
        }
    }

    fn workout() -> fitness::Workout {
        fitness::Workout {
            id: "fitness:2026-07-21T21:03:00".to_string(),
            path: "2026-07-21T17-03-00-04-00".to_string(),
            title: "Lift".to_string(),
            raw_title: "Lift".to_string(),
            started_at_local: "2026-07-21 17:03:00".to_string(),
            ended_at_local: "2026-07-21 18:03:00".to_string(),
            eastern_offset_minutes: -240,
            end_eastern_offset_minutes: -240,
            duration_seconds: 3_600,
            duration_suspicious: false,
            notes: None,
            description: None,
            sets: vec![set()],
        }
    }

    fn record(level: &str, kind: &str) -> fitness::Record {
        fitness::Record {
            level: level.to_string(),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn record_tags_annotate_only_genuine_records_and_collapse() {
        // Runner-up podium places stay wire data, never presentation.
        assert_eq!(
            record_label(&[record("silver", "1rm"), record("bronze", "reps")]),
            None,
        );
        // A set that led every category is plainly a PR.
        assert_eq!(
            record_label(&[
                record("gold", "1rm"),
                record("gold", "max-weight"),
                record("gold", "volume"),
                record("gold", "reps"),
            ])
            .as_deref(),
            Some("PR"),
        );
        // The estimated-1RM record subsumes the max-load record it rode in
        // with; a lone max-load record keeps its own name.
        assert_eq!(
            record_label(&[record("gold", "1rm"), record("gold", "max-weight")]).as_deref(),
            Some("PR: 1RM"),
        );
        assert_eq!(
            record_label(&[record("gold", "max-weight"), record("silver", "reps")]).as_deref(),
            Some("PR: max load"),
        );
        assert_eq!(
            record_label(&[record("gold", "volume"), record("gold", "reps")]).as_deref(),
            Some("PR: volume · reps"),
        );
    }

    #[test]
    fn pager_window_has_compact_gaps() {
        assert_eq!(
            page_window(6, 12),
            vec![
                Some(1),
                None,
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
                None,
                Some(12)
            ]
        );
        assert_eq!(
            page_window(1, 12),
            vec![Some(1), Some(2), Some(3), None, Some(12)]
        );
    }

    #[test]
    fn pager_counts_lifts_and_runs_but_not_interruptions() {
        let page = fitness::FitnessLogPage {
            page: 2,
            per_page: 10,
            total_sets: 31,
            total_lifts: 9,
            total_runs: 2,
            matching_runs: Vec::new(),
            activities: Vec::new(),
            interruptions: vec![fitness::Interruption {
                id: "travel".into(),
                from_date: "2026-08-01".into(),
                to_date: Some("2026-08-02".into()),
                note: "travel".into(),
                emoji: "✈️".into(),
                updated_at: 0,
            }],
        };
        assert_eq!(total_pages(&page), 2);
        let filters = Filters::normalize(vec![("page".into(), "2".into())]).unwrap();
        let pager = make_pager(&page, &filters).unwrap();
        assert_eq!(pager.current, 2);
        assert_eq!(pager.newer.as_deref(), Some("/fitness/log#set-log"));
        assert!(pager.older.is_none());
    }

    #[test]
    fn fallback_time_and_distance_are_not_repeated_in_details() {
        let mut timed = set();
        timed.set_time_seconds = Some(75);
        timed.superset_id = Some(1);
        assert_eq!(prescription(&timed), "1m 15s");
        assert_eq!(set_details(&timed), "");

        let mut distance = set();
        distance.distance_milli = Some(2_500);
        distance.set_time_seconds = Some(60);
        assert_eq!(prescription(&distance), "distance 2.5");
        assert_eq!(set_details(&distance), "1m 00s");
    }

    #[test]
    fn weighted_sets_show_pounds_and_repetitions() {
        let mut weighted = set();
        weighted.weight_milli = Some(95_000);
        weighted.reps = Some(5);
        assert_eq!(prescription(&weighted), "95 lbs × 5");

        weighted.reps = None;
        assert_eq!(prescription(&weighted), "95 lbs");

        weighted.weight_milli = Some(-45_500);
        weighted.reps = Some(8);
        assert_eq!(prescription(&weighted), "-45.5 lbs × 8");
    }

    #[test]
    fn adjacent_exercises_with_the_same_superset_share_one_block() {
        let mut normal = set();
        normal.exercise_name = "Press".to_string();

        let mut first = set();
        first.ordinal = 2;
        first.exercise_name = "Press".to_string();
        first.superset_id = Some(7);

        let mut second = set();
        second.ordinal = 3;
        second.exercise_name = "Row".to_string();
        second.superset_id = Some(7);

        let sets = [normal, first, second];
        let blocks = exercise_blocks(&sets, "lift");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].groups.len(), 1);
        assert_eq!(blocks[1].superset_id, Some(7));
        assert_eq!(blocks[1].groups.len(), 2);
    }

    #[test]
    fn workout_links_use_the_workers_canonical_public_path() {
        let workout = workout();
        let card = WorkoutCard::from(&workout);
        assert_eq!(card.href, "/fitness/lift/2026-07-21T17-03-00-04-00");
    }

    #[test]
    fn worker_paths_are_escaped_as_one_url_segment() {
        assert_eq!(
            workout_url("manual:abc 123"),
            "/fitness/lift/manual%3Aabc%20123"
        );
    }
}
