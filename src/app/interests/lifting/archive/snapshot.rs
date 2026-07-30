//! The in-memory archive snapshot: every read endpoint served from RAM.
//!
//! Built from three full-table loads whenever the data version changes
//! (an import — a few times a week), then immutable behind an `Arc`.
//! Records are derived here in one chronological pass. Filter semantics
//! mirror the old Worker's SQL exactly: ASCII-only case folding (SQLite
//! NOCASE), byte-order sorts, comparisons that exclude NULL, join-based
//! visibility (a set is only readable through its workout), and counts
//! that include every matching set while pages step over whole workouts.

use std::collections::HashMap;

use benjisponge::data::fitness_models::{ExerciseTag, LiftSet, Workout};

use super::api;
use super::eastern::{self, EasternInstant, InvalidTimestamp};
use super::filters::Filters;
use super::records::{self, SetSource};
use super::scoring;
use crate::app::interests::lifting::training_focus::{self, TrainingFocus, TrainingSet};

const PUBLISHED_WORKOUT_SOURCE: &str = "manual";

pub struct Snapshot {
    pub version: i64,
    workouts: Vec<SnapWorkout>,
    tags_by_exercise: HashMap<String, Vec<(String, String)>>,
    facets: api::Facets,
    calendar: api::Calendar,
    ids: api::SetIds,
}

/// One workout explicitly published by the authenticated manual-entry path.
/// This stays internal to the server: `source` and the UTC sort instant must
/// not widen the public fitness JSON contract.
#[derive(Clone, Debug)]
pub struct PublishedWorkout {
    pub workout: api::Workout,
    pub date: String,
    pub start_time: i64,
}

/// Compact fields the heatmap day popover needs — deliberately not a full
/// wire workout. Built by scanning set exercise names and volume points
/// without cloning set rows into the page payload.
#[derive(Clone, Debug)]
pub struct HeatmapWorkoutSummary {
    pub title: String,
    pub path: String,
    pub duration_seconds: u64,
    pub set_count: usize,
    pub volume_points: u32,
    pub exercises: Vec<String>,
}

struct SnapWorkout {
    /// Wire shape with `sets` left empty; responses clone it and fill in
    /// the sets the request's filters admit.
    wire: api::Workout,
    sets: Vec<SnapSet>,
    source: String,
    /// Epoch seconds from the source UTC instant, for ordering dynamic feed
    /// entries against Spire runs on the same Eastern date.
    start_time: i64,
    /// Stored projection columns — the filter/lookup side, exactly like
    /// the Worker's SQL reading `w.started_at_local`. (The wire side is
    /// re-derived from UTC, also exactly like the Worker.)
    row_local: String,
    row_offset: i64,
    local_date: String,
    hour: u8,
    /// 0 = Sunday, mirroring `strftime('%w', …)`.
    weekday: u8,
}

struct SnapSet {
    wire: api::Set,
    incomplete: bool,
}

pub fn build(
    version: i64,
    workout_rows: Vec<Workout>,
    set_rows: Vec<LiftSet>,
    tag_rows: Vec<ExerciseTag>,
) -> Result<Snapshot, InvalidTimestamp> {
    // `/api/fitness/ids` is the one read with no workout join: it sees
    // every stored set, sorted by id (byte order).
    let mut all_ids: Vec<String> = set_rows.iter().map(|row| row.id.clone()).collect();
    all_ids.sort_unstable();
    let ids = api::SetIds { ids: all_ids };

    let mut workout_rows = workout_rows;
    workout_rows.sort_unstable_by(|a, b| {
        b.started_at_utc
            .cmp(&a.started_at_utc)
            .then_with(|| b.id.cmp(&a.id))
    });
    let utc_by_workout: HashMap<&str, &str> = workout_rows
        .iter()
        .map(|row| (row.id.as_str(), row.started_at_utc.as_str()))
        .collect();

    // Records look at all sets in chronological order: workout UTC start,
    // then workout id, then ordinal. A set whose workout is missing is
    // unreadable through every joined endpoint, so it earns nothing and
    // beats nothing.
    let mut chronological: Vec<&LiftSet> = set_rows
        .iter()
        .filter(|row| utc_by_workout.contains_key(row.workout_id.as_str()))
        .collect();
    chronological.sort_unstable_by(|a, b| {
        let a_utc = utc_by_workout[a.workout_id.as_str()];
        let b_utc = utc_by_workout[b.workout_id.as_str()];
        a_utc
            .cmp(b_utc)
            .then_with(|| a.workout_id.cmp(&b.workout_id))
            .then_with(|| a.ordinal.cmp(&b.ordinal))
    });
    let badges = records::derive(chronological.iter().map(|row| SetSource {
        id: &row.id,
        exercise_name: &row.exercise_name,
        set_type: &row.set_type,
        weight_milli: row.weight_milli,
        reps: row.reps,
    }));

    let mut sets_by_workout: HashMap<&str, Vec<&LiftSet>> = HashMap::new();
    for row in &chronological {
        sets_by_workout
            .entry(row.workout_id.as_str())
            .or_default()
            .push(row);
    }

    let mut workouts = Vec::with_capacity(workout_rows.len());
    for row in &workout_rows {
        let start_time = eastern::utc_timestamp(&row.started_at_utc)?.as_second();
        let start = eastern::eastern_instant(&row.started_at_utc, 0)?;
        let end = eastern::eastern_instant(&row.started_at_utc, row.duration_seconds)?;
        let mut sets: Vec<&LiftSet> = sets_by_workout.remove(row.id.as_str()).unwrap_or_default();
        sets.sort_unstable_by_key(|set| set.ordinal);
        let sets = sets
            .into_iter()
            .map(|set| SnapSet {
                wire: api::Set {
                    id: set.id.clone(),
                    ordinal: to_u32(set.ordinal),
                    exercise_name: set.exercise_name.clone(),
                    raw_exercise_name: set.raw_exercise_name.clone(),
                    exercise_note: set.exercise_note.clone(),
                    superset_id: set.superset_id.map(to_u64),
                    weight_milli: set.weight_milli.map(to_u64),
                    weight_unit: set.weight_unit.clone(),
                    reps: set.reps.map(to_u64),
                    effort_hundredths: set.effort_hundredths.map(to_u64),
                    distance_milli: set.distance_milli.map(to_u64),
                    set_time_seconds: set.set_time_seconds.map(to_u64),
                    set_type: set.set_type.clone(),
                    records: badges
                        .get(&set.id)
                        .map(|earned| {
                            earned
                                .iter()
                                .map(|badge| api::Record {
                                    level: badge.level.as_str().to_string(),
                                    kind: badge.kind.as_str().to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                },
                incomplete: set.incomplete,
            })
            .collect();

        let local_date = row.started_at_local.get(..10).unwrap_or("").to_string();
        let hour = row
            .started_at_local
            .get(11..13)
            .and_then(|h| h.parse().ok())
            .unwrap_or(0);
        workouts.push(SnapWorkout {
            wire: api::Workout {
                id: row.id.clone(),
                path: eastern::public_path(&start),
                title: row.title.clone(),
                raw_title: row.raw_title.clone(),
                started_at_local: start.local.clone(),
                ended_at_local: end.local,
                eastern_offset_minutes: start.offset_minutes,
                end_eastern_offset_minutes: end.offset_minutes,
                duration_seconds: to_u64(row.duration_seconds),
                duration_suspicious: row.duration_suspicious,
                notes: row.notes.clone(),
                description: row.description.clone(),
                sets: Vec::new(),
            },
            sets,
            source: row.source.clone(),
            start_time,
            row_local: row.started_at_local.clone(),
            row_offset: row.eastern_offset_minutes,
            weekday: weekday_sunday_zero(&local_date),
            local_date,
            hour,
        });
    }

    let mut tags_by_exercise: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for tag in tag_rows {
        tags_by_exercise
            .entry(tag.exercise_name)
            .or_default()
            .push((tag.kind, tag.value));
    }

    let facets = build_facets(version, &workouts, &tags_by_exercise);
    let calendar = build_calendar(version, &workouts);

    Ok(Snapshot {
        version,
        workouts,
        tags_by_exercise,
        facets,
        calendar,
        ids,
    })
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn to_u32(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

fn weekday_sunday_zero(date: &str) -> u8 {
    let parse = || -> Option<jiff::civil::Date> {
        let year = date.get(..4)?.parse().ok()?;
        let month = date.get(5..7)?.parse().ok()?;
        let day = date.get(8..10)?.parse().ok()?;
        jiff::civil::Date::new(year, month, day).ok()
    };
    parse()
        .map(|date| date.weekday().to_sunday_zero_offset() as u8)
        .unwrap_or(0)
}

fn build_facets(
    version: i64,
    workouts: &[SnapWorkout],
    tags_by_exercise: &HashMap<String, Vec<(String, String)>>,
) -> api::Facets {
    let mut set_total = 0u64;
    let mut min_date: Option<&str> = None;
    let mut max_date: Option<&str> = None;
    let mut by_exercise: HashMap<&str, u64> = HashMap::new();
    let mut by_set_type: HashMap<&str, u64> = HashMap::new();
    let mut workouts_with_sets = 0u64;

    for workout in workouts {
        if workout.sets.is_empty() {
            continue;
        }
        workouts_with_sets += 1;
        set_total += workout.sets.len() as u64;
        let date = workout.local_date.as_str();
        if min_date.is_none_or(|current| date < current) {
            min_date = Some(date);
        }
        if max_date.is_none_or(|current| date > current) {
            max_date = Some(date);
        }
        for set in &workout.sets {
            *by_exercise
                .entry(set.wire.exercise_name.as_str())
                .or_default() += 1;
            *by_set_type.entry(set.wire.set_type.as_str()).or_default() += 1;
        }
    }

    // ORDER BY count DESC, value COLLATE NOCASE — ASCII-only case folding.
    let mut exercises: Vec<api::Facet> = by_exercise
        .iter()
        .map(|(value, count)| api::Facet {
            value: value.to_string(),
            count: *count,
        })
        .collect();
    exercises.sort_unstable_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| {
                a.value
                    .to_ascii_lowercase()
                    .cmp(&b.value.to_ascii_lowercase())
            })
            .then_with(|| a.value.cmp(&b.value))
    });

    // Tag facet counts join through sets: each tag counts the sets whose
    // exercise carries it.
    let mut by_tag: HashMap<(&str, &str), u64> = HashMap::new();
    for workout in workouts {
        for set in &workout.sets {
            if let Some(tags) = tags_by_exercise.get(&set.wire.exercise_name) {
                for (kind, value) in tags {
                    *by_tag.entry((kind.as_str(), value.as_str())).or_default() += 1;
                }
            }
        }
    }
    let mut tag_facets = api::TagFacets {
        movement: Vec::new(),
        muscle: Vec::new(),
        equipment: Vec::new(),
    };
    for ((kind, value), count) in by_tag {
        let bucket = match kind {
            "movement" => &mut tag_facets.movement,
            "muscle" => &mut tag_facets.muscle,
            "equipment" => &mut tag_facets.equipment,
            _ => continue,
        };
        bucket.push(api::Facet {
            value: value.to_string(),
            count,
        });
    }
    for bucket in [
        &mut tag_facets.movement,
        &mut tag_facets.muscle,
        &mut tag_facets.equipment,
    ] {
        bucket.sort_unstable_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    }

    let mut set_types: Vec<api::Facet> = by_set_type
        .iter()
        .map(|(value, count)| api::Facet {
            value: value.to_string(),
            count: *count,
        })
        .collect();
    set_types.sort_unstable_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));

    api::Facets {
        version,
        summary: api::Summary {
            sets: set_total,
            workouts: workouts_with_sets,
            min_date: min_date.map(str::to_string),
            max_date: max_date.map(str::to_string),
        },
        exercises,
        tags: tag_facets,
        set_types,
    }
}

fn build_calendar(version: i64, workouts: &[SnapWorkout]) -> api::Calendar {
    let mut by_date: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
    for workout in workouts {
        for set in &workout.sets {
            let points = scoring::set_volume_points(&set.wire.set_type, set.wire.effort_hundredths);
            *by_date.entry(workout.local_date.as_str()).or_default() += points;
        }
    }
    api::Calendar {
        version,
        days: by_date
            .into_iter()
            .map(|(date, volume_points)| api::CalendarDay {
                date: date.to_string(),
                volume_points,
            })
            .collect(),
    }
}

impl Snapshot {
    /// Full manual workouts for the homepage timeline and `/feed.xml`,
    /// newest-first. CSV imports remain archive-only; publishing one must
    /// never dump the historical corpus into subscribers' readers.
    pub fn published_workouts(&self) -> Vec<PublishedWorkout> {
        self.workouts
            .iter()
            .filter(|snap| snap.source == PUBLISHED_WORKOUT_SOURCE)
            .map(|snap| {
                let mut workout = snap.wire.clone();
                workout.sets = snap.sets.iter().map(|set| set.wire.clone()).collect();
                PublishedWorkout {
                    date: workout.started_at_local[..10].to_string(),
                    start_time: snap.start_time,
                    workout,
                }
            })
            .collect()
    }

    pub fn facets(&self) -> api::Facets {
        self.facets.clone()
    }

    pub fn calendar(&self) -> api::Calendar {
        self.calendar.clone()
    }

    /// Page-only rolling muscle load and next-focus guidance. It is derived
    /// at request time because "the last seven days" advances even when no
    /// import rebuilds this immutable snapshot.
    pub(in crate::app::interests::lifting) fn training_focus(
        &self,
        today: jiff::civil::Date,
    ) -> TrainingFocus {
        training_focus::derive(
            self.workouts.iter().flat_map(|workout| {
                workout.sets.iter().map(|set| TrainingSet {
                    date: workout.local_date.as_str(),
                    exercise_name: set.wire.exercise_name.as_str(),
                    set_type: set.wire.set_type.as_str(),
                    effort_hundredths: set.wire.effort_hundredths,
                    tags: self
                        .tags_by_exercise
                        .get(&set.wire.exercise_name)
                        .map(Vec::as_slice),
                })
            }),
            today,
        )
    }

    /// Daily volume points over only the sets a filter admits, using the
    /// same per-set predicate as `sets_page` so the log page's heatmap
    /// and its set log can never disagree about what matched. Pagination
    /// fields are irrelevant here and ignored. With an empty filter this
    /// reproduces `calendar()` exactly, including zero-point warm-up-only
    /// days.
    pub fn calendar_filtered(&self, filters: &Filters) -> api::Calendar {
        let needle = filters.q.as_ref().map(|q| q.to_ascii_lowercase());
        let mut by_date: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
        for workout in &self.workouts {
            for set in &workout.sets {
                if self.matches(filters, needle.as_deref(), workout, set) {
                    let points =
                        scoring::set_volume_points(&set.wire.set_type, set.wire.effort_hundredths);
                    *by_date.entry(workout.local_date.as_str()).or_default() += points;
                }
            }
        }
        api::Calendar {
            version: self.version,
            days: by_date
                .into_iter()
                .map(|(date, volume_points)| api::CalendarDay {
                    date: date.to_string(),
                    volume_points,
                })
                .collect(),
        }
    }

    /// Movement/muscle/equipment tags keyed by canonical exercise name —
    /// the same map the tag filters consult. Pages use it to derive the
    /// muscle involvement a workout page displays; it never rides the
    /// public wire.
    pub fn exercise_tag_map(&self) -> &HashMap<String, Vec<(String, String)>> {
        &self.tags_by_exercise
    }

    /// Compact heatmap-day summaries for one Eastern local date, newest-first.
    /// Page-only — the day popover shard loads these on demand so the calendar
    /// SSR never clones set payloads. Never rides the public calendar JSON.
    pub fn workouts_on_date(&self, date: &str) -> Vec<HeatmapWorkoutSummary> {
        self.workouts
            .iter()
            .filter(|snap| snap.local_date == date)
            .map(|snap| {
                let mut seen = std::collections::BTreeSet::new();
                let mut exercises = Vec::new();
                let mut volume_points = 0_u32;
                for set in &snap.sets {
                    volume_points = volume_points.saturating_add(scoring::set_volume_points(
                        set.wire.set_type.as_str(),
                        set.wire.effort_hundredths,
                    ));
                    if seen.insert(set.wire.exercise_name.as_str()) {
                        exercises.push(set.wire.exercise_name.clone());
                    }
                }
                HeatmapWorkoutSummary {
                    title: snap.wire.title.clone(),
                    path: snap.wire.path.clone(),
                    duration_seconds: snap.wire.duration_seconds,
                    set_count: snap.sets.len(),
                    volume_points,
                    exercises,
                }
            })
            .collect()
    }

    pub fn ids(&self) -> api::SetIds {
        self.ids.clone()
    }

    pub fn sets_page(&self, filters: &Filters) -> api::SetPage {
        let needle = filters.q.as_ref().map(|q| q.to_ascii_lowercase());
        let mut total_sets = 0u64;
        let mut matching: Vec<(usize, Vec<usize>)> = Vec::new();
        for (workout_index, workout) in self.workouts.iter().enumerate() {
            let mut set_indexes = Vec::new();
            for (set_index, set) in workout.sets.iter().enumerate() {
                if self.matches(filters, needle.as_deref(), workout, set) {
                    set_indexes.push(set_index);
                }
            }
            if !set_indexes.is_empty() {
                total_sets += set_indexes.len() as u64;
                matching.push((workout_index, set_indexes));
            }
        }
        let total_workouts = matching.len() as u64;
        let workouts = matching
            .iter()
            .skip((filters.page - 1) * filters.per_page)
            .take(filters.per_page)
            .map(|(workout_index, set_indexes)| {
                let snap = &self.workouts[*workout_index];
                let mut wire = snap.wire.clone();
                wire.sets = set_indexes
                    .iter()
                    .map(|index| snap.sets[*index].wire.clone())
                    .collect();
                wire
            })
            .collect();
        api::SetPage {
            version: self.version,
            page: filters.page,
            per_page: filters.per_page,
            total_sets,
            total_workouts,
            workouts,
        }
    }

    pub fn latest(&self) -> api::WorkoutDetail {
        if self.workouts.is_empty() {
            return api::WorkoutDetail {
                version: self.version,
                workout: None,
                newer_workout_path: None,
                older_workout_path: None,
            };
        }
        self.detail(0)
    }

    pub fn by_path(&self, instant: &EasternInstant) -> Option<api::WorkoutDetail> {
        let index = self.workouts.iter().position(|workout| {
            workout.row_local == instant.local
                && workout.row_offset == i64::from(instant.offset_minutes)
        })?;
        Some(self.detail(index))
    }

    /// The full workout at `index` plus its chronological neighbors —
    /// `index` is into the newest-first ordering, so "newer" is the
    /// previous element.
    fn detail(&self, index: usize) -> api::WorkoutDetail {
        let snap = &self.workouts[index];
        let mut wire = snap.wire.clone();
        wire.sets = snap.sets.iter().map(|set| set.wire.clone()).collect();
        api::WorkoutDetail {
            version: self.version,
            workout: Some(wire),
            newer_workout_path: index
                .checked_sub(1)
                .map(|newer| self.workouts[newer].wire.path.clone()),
            older_workout_path: self
                .workouts
                .get(index + 1)
                .map(|older| older.wire.path.clone()),
        }
    }

    fn matches(
        &self,
        filters: &Filters,
        needle: Option<&str>,
        workout: &SnapWorkout,
        set: &SnapSet,
    ) -> bool {
        if let Some(from) = &filters.from
            && workout.local_date.as_str() < from.as_str()
        {
            return false;
        }
        if let Some(to) = &filters.to
            && workout.local_date.as_str() > to.as_str()
        {
            return false;
        }
        if let Some(band) = filters.time_of_day
            && !band.contains_hour(workout.hour)
        {
            return false;
        }
        if let Some(weekday) = filters.weekday
            && workout.weekday != weekday
        {
            return false;
        }
        if let Some(suspicious) = filters.duration_suspicious
            && workout.wire.duration_suspicious != suspicious
        {
            return false;
        }
        if !filters.set_types.is_empty() && !filters.set_types.contains(&set.wire.set_type) {
            return false;
        }
        if let Some(exercise) = &filters.exercise
            && !set.wire.exercise_name.eq_ignore_ascii_case(exercise)
        {
            return false;
        }
        // Numeric comparisons exclude NULL, like SQL.
        if let Some(min) = filters.min_load
            && !set.wire.weight_milli.is_some_and(|weight| weight >= min)
        {
            return false;
        }
        if let Some(max) = filters.max_load
            && !set.wire.weight_milli.is_some_and(|weight| weight <= max)
        {
            return false;
        }
        if let Some(min) = filters.min_reps
            && !set.wire.reps.is_some_and(|reps| reps >= min)
        {
            return false;
        }
        if let Some(max) = filters.max_reps
            && !set.wire.reps.is_some_and(|reps| reps <= max)
        {
            return false;
        }
        if let Some(max) = filters.max_effort
            && !set
                .wire
                .effort_hundredths
                .is_some_and(|effort| effort <= max)
        {
            return false;
        }
        if let Some(wanted) = filters.has_record
            && !set.wire.records.is_empty() != wanted
        {
            return false;
        }
        if let Some(wanted) = filters.has_superset
            && set.wire.superset_id.is_some() != wanted
        {
            return false;
        }
        if let Some(wanted) = filters.has_notes {
            let has = workout.wire.notes.is_some()
                || workout.wire.description.is_some()
                || set.wire.exercise_note.is_some();
            if has != wanted {
                return false;
            }
        }
        if let Some(wanted) = filters.incomplete
            && set.incomplete != wanted
        {
            return false;
        }
        for (selected, kind) in [
            (&filters.movement, "movement"),
            (&filters.muscle, "muscle"),
            (&filters.equipment, "equipment"),
        ] {
            if selected.is_empty() {
                continue;
            }
            let tags = self.tags_by_exercise.get(&set.wire.exercise_name);
            let hit = tags.is_some_and(|tags| {
                tags.iter()
                    .any(|(tag_kind, value)| tag_kind == kind && selected.contains(value))
            });
            if !hit {
                return false;
            }
        }
        if let Some(needle) = needle {
            let haystacks = [
                Some(workout.wire.title.as_str()),
                Some(workout.wire.raw_title.as_str()),
                workout.wire.notes.as_deref(),
                workout.wire.description.as_deref(),
                Some(set.wire.exercise_name.as_str()),
                Some(set.wire.raw_exercise_name.as_str()),
                set.wire.exercise_note.as_deref(),
            ];
            let hit = haystacks
                .into_iter()
                .flatten()
                .any(|haystack| ascii_ci_contains(haystack, needle));
            if !hit {
                return false;
            }
        }
        true
    }
}

/// ASCII-case-insensitive substring — SQLite `LIKE '%…%' COLLATE NOCASE`.
/// The needle is already lowercased; only A–Z fold, like NOCASE.
fn ascii_ci_contains(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return true;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::super::filters::parse_filters;
    use super::*;

    fn workout_row(id_stamp: &str, utc: &str, local: &str, offset: i64) -> Workout {
        Workout {
            id: format!("fitness:{id_stamp}"),
            title: "Leg day".into(),
            raw_title: "Leg day!".into(),
            started_at_utc: utc.into(),
            started_at_local: local.into(),
            eastern_offset_minutes: offset,
            duration_seconds: 3600,
            duration_suspicious: false,
            notes: None,
            description: None,
            source: "workout-data-csv".into(),
            imported_at: 0,
        }
    }

    fn set_row(
        workout_stamp: &str,
        ordinal: i64,
        exercise: &str,
        weight: Option<i64>,
        reps: Option<i64>,
    ) -> LiftSet {
        LiftSet {
            id: format!("fitness:{workout_stamp}:{ordinal:04}"),
            workout_id: format!("fitness:{workout_stamp}"),
            exercise_name: exercise.into(),
            raw_exercise_name: exercise.into(),
            ordinal,
            exercise_note: None,
            superset_id: None,
            weight_milli: weight,
            weight_unit: "lbs".into(),
            reps,
            effort_hundredths: Some(800),
            distance_milli: None,
            set_time_seconds: None,
            set_type: "NORMAL_SET".into(),
            incomplete: reps.is_none(),
        }
    }

    fn snapshot() -> Snapshot {
        // Two workouts, EDT; newest first everywhere in the output.
        let workouts = vec![
            workout_row(
                "2026-07-20T14:00:00",
                "2026-07-20 14:00:00",
                "2026-07-20 10:00:00",
                -240,
            ),
            workout_row(
                "2026-07-21T14:39:04",
                "2026-07-21 14:39:04",
                "2026-07-21 10:39:04",
                -240,
            ),
        ];
        let sets = vec![
            set_row(
                "2026-07-20T14:00:00",
                1,
                "Squat (Barbell)",
                Some(200_000),
                Some(5),
            ),
            set_row(
                "2026-07-21T14:39:04",
                1,
                "Squat (Barbell)",
                Some(225_000),
                Some(5),
            ),
            set_row("2026-07-21T14:39:04", 2, "Bench Press", Some(135_000), None),
        ];
        let tags = vec![
            ExerciseTag {
                exercise_name: "Squat (Barbell)".into(),
                kind: "movement".into(),
                value: "squat-type".into(),
            },
            ExerciseTag {
                exercise_name: "Squat (Barbell)".into(),
                kind: "muscle".into(),
                value: "quads".into(),
            },
        ];
        build(7, workouts, sets, tags).unwrap()
    }

    #[test]
    fn newest_first_with_derived_paths_and_frozen_records() {
        let snap = snapshot();
        let page = snap.sets_page(&parse_filters(&[]).unwrap());
        assert_eq!(page.version, 7);
        assert_eq!(page.total_sets, 3);
        assert_eq!(page.total_workouts, 2);
        assert_eq!(page.workouts[0].path, "2026-07-21T10-39-04-04-00");
        assert_eq!(page.workouts[1].path, "2026-07-20T10-00-00-04-00");
        // The first-ever squat is gold everywhere; the heavier later squat
        // takes gold at its own time (history is frozen, not rewritten).
        let first = &page.workouts[1].sets[0];
        let later = &page.workouts[0].sets[0];
        assert!(first.records.iter().any(|r| r.level == "gold"));
        assert!(
            later
                .records
                .iter()
                .any(|r| r.level == "gold" && r.kind == "1rm")
        );
    }

    #[test]
    fn filters_page_by_whole_workouts_and_reapply_to_sets() {
        let snap = snapshot();
        let filters = parse_filters(&[("movement".into(), "squat-type".into())]).unwrap();
        let page = snap.sets_page(&filters);
        assert_eq!(page.total_sets, 2, "bench set filtered out");
        assert_eq!(page.total_workouts, 2);
        assert_eq!(
            page.workouts[0].sets.len(),
            1,
            "workout keeps only its matching sets"
        );

        let q = parse_filters(&[("q".into(), "BENCH".into())]).unwrap();
        let page = snap.sets_page(&q);
        assert_eq!(page.total_sets, 1, "NOCASE substring");
        assert_eq!(page.total_workouts, 1);

        let incomplete = parse_filters(&[("incomplete".into(), "true".into())]).unwrap();
        assert_eq!(snap.sets_page(&incomplete).total_sets, 1);
    }

    #[test]
    fn facets_calendar_ids_and_neighbors() {
        let snap = snapshot();
        let facets = snap.facets();
        assert_eq!(facets.summary.sets, 3);
        assert_eq!(facets.summary.workouts, 2);
        assert_eq!(facets.summary.min_date.as_deref(), Some("2026-07-20"));
        assert_eq!(facets.summary.max_date.as_deref(), Some("2026-07-21"));
        assert_eq!(facets.exercises[0].value, "Squat (Barbell)");
        assert_eq!(facets.exercises[0].count, 2);
        assert_eq!(facets.tags.movement[0].value, "squat-type");
        assert_eq!(facets.tags.movement[0].count, 2, "tag counts join sets");

        let calendar = snap.calendar();
        assert_eq!(calendar.days.len(), 2);
        assert_eq!(calendar.days[0].date, "2026-07-20", "ascending dates");
        assert_eq!(calendar.days[0].volume_points, 3, "one RPE-8 set");

        assert_eq!(snap.ids().ids.len(), 3);

        let latest = snap.latest();
        let workout = latest.workout.unwrap();
        assert_eq!(workout.path, "2026-07-21T10-39-04-04-00");
        assert_eq!(latest.newer_workout_path, None);
        assert_eq!(
            latest.older_workout_path.as_deref(),
            Some("2026-07-20T10-00-00-04-00")
        );

        let by_path = snap
            .by_path(&EasternInstant {
                local: "2026-07-20 10:00:00".into(),
                offset_minutes: -240,
            })
            .unwrap();
        assert_eq!(
            by_path.newer_workout_path.as_deref(),
            Some("2026-07-21T10-39-04-04-00")
        );
        assert_eq!(by_path.older_workout_path, None);
        assert!(
            snap.by_path(&EasternInstant {
                local: "2026-07-20 10:00:00".into(),
                offset_minutes: -300,
            })
            .is_none(),
            "offset participates in identity"
        );
    }

    #[test]
    fn weekday_and_time_bands_read_the_local_projection() {
        let snap = snapshot();
        // 2026-07-21 is a Tuesday; local hour is 10 (morning).
        let tue = parse_filters(&[("weekday".into(), "tue".into())]).unwrap();
        assert_eq!(snap.sets_page(&tue).total_workouts, 1);
        let sun = parse_filters(&[("weekday".into(), "sun".into())]).unwrap();
        assert_eq!(snap.sets_page(&sun).total_workouts, 0);
        let morning = parse_filters(&[("time_of_day".into(), "morning".into())]).unwrap();
        assert_eq!(snap.sets_page(&morning).total_workouts, 2);
        let night = parse_filters(&[("time_of_day".into(), "night".into())]).unwrap();
        assert_eq!(snap.sets_page(&night).total_workouts, 0);
    }

    #[test]
    fn filtered_calendar_mirrors_the_set_predicate() {
        let snap = snapshot();

        // Empty filter reproduces the precomputed calendar exactly.
        let all = snap.calendar_filtered(&parse_filters(&[]).unwrap());
        let precomputed = snap.calendar();
        assert_eq!(all.version, precomputed.version);
        assert_eq!(all.days.len(), precomputed.days.len());
        for (filtered, unfiltered) in all.days.iter().zip(precomputed.days.iter()) {
            assert_eq!(filtered.date, unfiltered.date);
            assert_eq!(filtered.volume_points, unfiltered.volume_points);
        }
        assert_eq!(all.days[1].volume_points, 6, "two RPE-8 sets that day");

        // A set-level filter drops only the non-matching sets' points.
        let squats = snap.calendar_filtered(
            &parse_filters(&[("movement".into(), "squat-type".into())]).unwrap(),
        );
        assert_eq!(squats.days.len(), 2, "both days still have a squat set");
        assert_eq!(squats.days[0].date, "2026-07-20");
        assert_eq!(squats.days[0].volume_points, 3);
        assert_eq!(
            squats.days[1].volume_points, 3,
            "the bench set no longer scores"
        );

        // A day with zero matching sets disappears entirely.
        let bench =
            snap.calendar_filtered(&parse_filters(&[("q".into(), "bench".into())]).unwrap());
        assert_eq!(bench.days.len(), 1);
        assert_eq!(bench.days[0].date, "2026-07-21");
        assert_eq!(bench.days[0].volume_points, 3);
    }

    #[test]
    fn exercise_tag_map_exposes_the_filter_side_tags() {
        let snap = snapshot();
        let tags = snap.exercise_tag_map();
        let squat = tags.get("Squat (Barbell)").expect("tagged exercise");
        assert!(squat.contains(&("movement".to_string(), "squat-type".to_string())));
        assert!(squat.contains(&("muscle".to_string(), "quads".to_string())));
        assert!(
            !tags.contains_key("Bench Press"),
            "untagged exercise absent"
        );
    }

    #[test]
    fn workouts_on_date_keeps_exercise_order_without_set_payloads() {
        let snap = snapshot();
        let day = snap.workouts_on_date("2026-07-21");
        assert_eq!(day.len(), 1);
        assert_eq!(
            day[0].exercises,
            vec!["Squat (Barbell)".to_string(), "Bench Press".to_string()]
        );
        assert_eq!(day[0].set_count, 2);
        assert!(day[0].volume_points > 0);
        assert!(snap.workouts_on_date("1999-01-01").is_empty());
    }

    #[test]
    fn only_manual_workouts_are_projected_for_the_feed() {
        let mut manual = workout_row(
            "2026-07-21T14:39:04",
            "2026-07-21 14:39:04",
            "2026-07-21 10:39:04",
            -240,
        );
        manual.source = "manual".into();
        let csv = workout_row(
            "2026-07-20T14:00:00",
            "2026-07-20 14:00:00",
            "2026-07-20 10:00:00",
            -240,
        );
        let sets = vec![
            set_row(
                "2026-07-21T14:39:04",
                1,
                "Incline Bench Press",
                Some(135_000),
                Some(6),
            ),
            set_row(
                "2026-07-20T14:00:00",
                1,
                "Squat (Barbell)",
                Some(225_000),
                Some(5),
            ),
        ];
        let snap = build(8, vec![csv, manual], sets, Vec::new()).unwrap();

        let published = snap.published_workouts();
        assert_eq!(published.len(), 1, "CSV history stays out of the feed");
        assert_eq!(published[0].workout.id, "fitness:2026-07-21T14:39:04");
        assert_eq!(published[0].workout.sets.len(), 1);
        assert_eq!(published[0].date, "2026-07-21");
        assert_eq!(
            published[0].start_time,
            eastern::utc_timestamp("2026-07-21 14:39:04")
                .unwrap()
                .as_second()
        );
    }

    #[test]
    fn out_of_range_pages_are_empty_but_counted() {
        let snap = snapshot();
        let mut filters = parse_filters(&[]).unwrap();
        filters.page = 99;
        let page = snap.sets_page(&filters);
        assert_eq!(page.total_sets, 3);
        assert!(page.workouts.is_empty());
    }
}
