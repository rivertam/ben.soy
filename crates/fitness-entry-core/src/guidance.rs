use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::MAX_WEIGHT_MILLI;
use crate::draft::{ActionError, Draft, DraftSet};
use crate::muscle_load::{MuscleDeltas, dot_product, remaining_deltas};
use crate::text::{
    SetType, effort_to_hundredths, hundredths_text, js_trim, pounds_to_milli, reps_value,
};

const COVERAGE_LIMIT: usize = 4;
const SEARCH_LIMIT: usize = 6;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideConfig {
    pub version: i64,
    pub today: String,
    pub weekly_pace_tenths: usize,
    /// Complete signed target-minus-current vector on the shared baseline scale.
    pub muscle_needs: MuscleDeltas,
    pub exercises: Vec<ExerciseGuide>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExerciseGuide {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub equipment: Vec<String>,
    pub bodyweight: bool,
    /// Coarse fatigue metadata used only to break equal-fit ties.
    #[serde(default)]
    pub high_fatigue: bool,
    #[serde(default)]
    pub high_axial_load: bool,
    pub last_date: String,
    pub set_count: usize,
    pub workout_count: usize,
    pub muscles: Vec<(String, u32)>,
    pub movements: Vec<String>,
    pub coarse_muscles: Vec<String>,
    pub marks: Vec<GuideMark>,
    pub loads: Vec<LoadPreset>,
    pub picker_meta: String,
    pub picker_mark: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideMark {
    pub kind: String,
    pub value: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LoadPreset {
    pub label: String,
    pub weight_milli: Option<i64>,
    pub set_type: SetType,
    pub display: String,
    pub spoken: String,
}

impl LoadPreset {
    pub fn new(
        label: &str,
        weight_milli: Option<i64>,
        set_type: SetType,
        bodyweight: bool,
    ) -> Self {
        Self {
            label: label.to_string(),
            weight_milli,
            set_type,
            display: load_display(weight_milli, bodyweight),
            spoken: load_spoken(weight_milli, bodyweight),
        }
    }
}

impl GuideConfig {
    pub fn validate(&self) -> Result<(), ActionError> {
        if self.today.parse::<jiff::civil::Date>().is_err() {
            return Err(ActionError::message(
                "The server supplied an invalid Fitness guide date.",
            ));
        }
        let mut names = HashSet::new();
        for exercise in &self.exercises {
            if exercise.name.is_empty() || !names.insert(exercise.name.as_str()) {
                return Err(ActionError::message(
                    "The server supplied an invalid Fitness exercise guide.",
                ));
            }
            if !exercise.last_date.is_empty()
                && exercise.last_date.parse::<jiff::civil::Date>().is_err()
            {
                return Err(ActionError::message(
                    "The server supplied invalid Fitness exercise history.",
                ));
            }
            if exercise
                .muscles
                .iter()
                .any(|(name, ratio)| name.is_empty() || !(1..=100).contains(ratio))
                || exercise.loads.iter().any(|load| {
                    load.weight_milli.is_some_and(|weight| {
                        !(-MAX_WEIGHT_MILLI..=MAX_WEIGHT_MILLI).contains(&weight)
                    })
                })
            {
                return Err(ActionError::message(
                    "The server supplied invalid Fitness recommendation data.",
                ));
            }
        }
        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.exercises.iter().any(|exercise| exercise.name == name)
    }

    fn exercise(&self, name: &str) -> Option<&ExerciseGuide> {
        self.exercises.iter().find(|exercise| exercise.name == name)
    }
}

#[cfg(test)]
impl ExerciseGuide {
    pub(crate) fn fixture(name: &str) -> Self {
        Self {
            aliases: Vec::new(),
            equipment: Vec::new(),
            name: name.into(),
            bodyweight: false,
            high_fatigue: false,
            high_axial_load: false,
            last_date: "2026-08-20".into(),
            set_count: 20,
            workout_count: 4,
            muscles: vec![("quads".into(), 100)],
            movements: vec!["squat-type".into()],
            coarse_muscles: vec!["legs".into()],
            marks: Vec::new(),
            loads: Vec::new(),
            picker_meta: "legs · 4 workouts · last 2026-08-20".into(),
            picker_mark: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GuidanceContext {
    pub direction: String,
    pub query: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Derived {
    pub exercise_count: usize,
    pub set_count: usize,
    pub completed_count: usize,
    pub total_rows: usize,
    pub unfinished_rows: usize,
    pub finish_enabled: bool,
    pub has_completed_set: bool,
    pub has_active_exercise: bool,
    pub coverage: Vec<Coverage>,
    pub starters: Vec<Suggestion>,
    pub deepen: Option<Suggestion>,
    pub expand: Option<Suggestion>,
    pub search: Vec<SearchHit>,
    pub search_feedback: String,
    pub quick_empty: String,
    pub set_views: Vec<SetView>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Coverage {
    pub muscle: String,
    pub label: String,
    pub level: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub lane: String,
    pub label: String,
    pub reason: String,
    pub mark: String,
    pub aria_label: String,
    pub score: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchHit {
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetView {
    pub id: String,
    pub weight_valid: bool,
    pub weight_milli: Option<i64>,
    pub reps_valid: bool,
    pub effort_valid: bool,
    pub effort_hundredths: Option<u64>,
    pub failure: bool,
    pub rir_display: String,
    pub rir_spoken: String,
    pub set_type_label: String,
    pub set_type_spoken: String,
    pub set_kind: String,
    pub can_complete: bool,
    pub volume_points: u32,
}

pub fn derive(draft: &Draft, guide: &GuideConfig, context: &GuidanceContext) -> Derived {
    let set_views: Vec<SetView> = draft
        .exercises
        .iter()
        .flat_map(|exercise| exercise.sets.iter().map(set_view))
        .collect();
    let views: BTreeMap<&str, &SetView> = set_views
        .iter()
        .map(|view| (view.id.as_str(), view))
        .collect();
    let set_count = set_views.iter().filter(|view| view.reps_valid).count();
    let completed_count = draft
        .exercises
        .iter()
        .flat_map(|exercise| &exercise.sets)
        .filter(|set| {
            set.done
                && views
                    .get(set.id.as_str())
                    .is_some_and(|view| view.can_complete)
        })
        .count();
    let total_rows = set_views.len();
    let session = session_context(draft, guide, &views);
    let coverage = coverage(&session);
    let query = js_trim(&context.query).to_lowercase();
    let has_completed_set = completed_count > 0;
    let has_active_exercise = !draft.exercises.is_empty();
    let starters = if !has_active_exercise && query.is_empty() {
        starter_suggestions(draft, guide, &context.direction)
    } else {
        Vec::new()
    };
    let (deepen, expand) = if has_active_exercise {
        next_suggestions(draft, guide, &session)
    } else {
        (None, None)
    };
    let (search, search_feedback) = search(draft, guide, &query);
    let quick_empty = if !query.is_empty() && search.is_empty() {
        "No matches.".to_string()
    } else if !has_active_exercise
        && !context.direction.is_empty()
        && query.is_empty()
        && starters.is_empty()
    {
        format!("No unused {} exercises.", context.direction)
    } else {
        String::new()
    };
    Derived {
        exercise_count: draft.exercises.len(),
        set_count,
        completed_count,
        total_rows,
        unfinished_rows: total_rows.saturating_sub(completed_count),
        finish_enabled: completed_count > 0,
        has_completed_set,
        has_active_exercise,
        coverage,
        starters,
        deepen,
        expand,
        search,
        search_feedback,
        quick_empty,
        set_views,
    }
}

fn set_view(set: &DraftSet) -> SetView {
    let weight_blank = js_trim(&set.weight).is_empty();
    let weight_milli = pounds_to_milli(&set.weight);
    let weight_valid = weight_blank || weight_milli.is_some();
    let reps_valid = reps_value(&set.reps).is_some();
    let effort_blank = js_trim(&set.effort).is_empty();
    let effort_hundredths = effort_to_hundredths(&set.effort);
    let effort_valid = (effort_blank || effort_hundredths.is_some())
        && !(set.failure && effort_hundredths.is_some());
    let (mut rir_display, mut rir_spoken) = match (set.failure, effort_blank, effort_hundredths) {
        (true, true, _) => ("FAIL".to_string(), "Reached failure".to_string()),
        (true, false, _) => ("?".to_string(), "Invalid failure effort".to_string()),
        (false, true, _) => ("—".to_string(), "Not rated".to_string()),
        (false, false, Some(effort)) => {
            let rir = 1_000 - effort;
            (hundredths_text(rir), rir_spoken(effort))
        }
        (false, false, None) => ("?".to_string(), "Invalid reps in reserve".to_string()),
    };
    if set.set_type == SetType::Warmup {
        rir_display = "WARM".to_string();
        rir_spoken = "Warm up".to_string();
    }
    SetView {
        id: set.id.clone(),
        weight_valid,
        weight_milli,
        reps_valid,
        effort_valid,
        effort_hundredths,
        failure: set.failure,
        rir_display,
        rir_spoken,
        set_type_label: set.set_type.short_label().to_string(),
        set_type_spoken: set.set_type.spoken_label().to_string(),
        set_kind: set.set_type.kind().to_string(),
        can_complete: weight_valid && reps_valid && effort_valid,
        volume_points: set_volume_points(set.set_type, effort_hundredths, set.failure),
    }
}

pub fn set_volume_points(set_type: SetType, effort_hundredths: Option<u64>, failure: bool) -> u32 {
    match set_type {
        SetType::Warmup => 0,
        _ if failure => 6,
        _ => match effort_hundredths {
            Some(1_000) => 5,
            Some(900) => 4,
            Some(800) => 3,
            _ => 2,
        },
    }
}

fn rir_spoken(effort: u64) -> String {
    let rir = 1_000 - effort;
    match rir {
        50 => "Half a rep in reserve".to_string(),
        100 => "1 rep in reserve".to_string(),
        _ => format!("{} reps in reserve", hundredths_text(rir)),
    }
}

#[derive(Default)]
struct SessionContext {
    muscle_load: BTreeMap<String, u32>,
    muscles: BTreeSet<String>,
    high_fatigue_count: usize,
    high_axial_count: usize,
    high_fatigue_movements: BTreeSet<String>,
    high_fatigue_coarse: BTreeSet<String>,
}

fn session_context(
    draft: &Draft,
    guide: &GuideConfig,
    views: &BTreeMap<&str, &SetView>,
) -> SessionContext {
    let mut context = SessionContext::default();
    for exercise in &draft.exercises {
        let Some(item) = guide.exercise(&exercise.name) else {
            continue;
        };
        let active: Vec<&SetView> = exercise
            .sets
            .iter()
            .filter_map(|set| views.get(set.id.as_str()).copied())
            .collect();
        if item.high_fatigue {
            context.high_fatigue_count += 1;
            context
                .high_fatigue_movements
                .extend(item.movements.iter().cloned());
            context
                .high_fatigue_coarse
                .extend(item.coarse_muscles.iter().cloned());
        }
        if item.high_axial_load {
            context.high_axial_count += 1;
        }
        // A selected exercise affects recommendations immediately, and every
        // added row increases its planned session dose. Blank working rows use
        // the archive's unrated two-point baseline; warm-ups remain zero.
        let volume: u32 = active.iter().map(|view| view.volume_points).sum();
        for (muscle, ratio) in &item.muscles {
            *context.muscle_load.entry(muscle.clone()).or_default() +=
                volume.saturating_mul(*ratio);
            context.muscles.insert(muscle.clone());
        }
    }
    context
}

fn coverage(context: &SessionContext) -> Vec<Coverage> {
    let mut ranked: Vec<(&String, &u32)> = context.muscle_load.iter().collect();
    ranked.sort_by(|(left_name, left), (right_name, right)| {
        right.cmp(left).then_with(|| left_name.cmp(right_name))
    });
    let maximum = ranked.first().map(|(_, value)| **value).unwrap_or(1).max(1);
    ranked
        .into_iter()
        .take(COVERAGE_LIMIT)
        .map(|(muscle, value)| {
            let ratio = f64::from(*value) / f64::from(maximum);
            Coverage {
                muscle: muscle.clone(),
                label: muscle_label(muscle),
                level: if ratio >= 0.72 {
                    "main"
                } else if ratio >= 0.35 {
                    "support"
                } else {
                    "touch"
                }
                .to_string(),
            }
        })
        .collect()
}

fn direction_movements(direction: &str) -> &'static [&'static str] {
    match direction {
        "push" => &["horizontal-push", "vertical-push", "dip"],
        "pull" => &["horizontal-pull", "vertical-pull", "shoulder-extension"],
        "squat" => &["squat-type"],
        "hinge" => &["hinge"],
        "arms" => &["elbow-flexion", "elbow-extension"],
        "shoulders" => &[
            "vertical-push",
            "shoulder-abduction",
            "shoulder-flexion",
            "rear-delt",
        ],
        _ => &[],
    }
}

fn starter_suggestions(draft: &Draft, guide: &GuideConfig, direction: &str) -> Vec<Suggestion> {
    let movements = direction_movements(direction);
    if movements.is_empty() {
        return Vec::new();
    }
    let selected: HashSet<&str> = draft
        .exercises
        .iter()
        .map(|exercise| exercise.name.as_str())
        .collect();
    let candidates: Vec<&ExerciseGuide> = guide
        .exercises
        .iter()
        .filter(|item| !selected.contains(item.name.as_str()))
        .filter(|item| {
            item.movements
                .iter()
                .any(|movement| movements.contains(&movement.as_str()))
        })
        .collect();
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter(|item| !item.muscles.is_empty())
        .map(|item| (item, dot_product(&guide.muscle_needs, &item.muscles)))
        .collect();
    ranked.sort_by(|(left, left_score), (right, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.workout_count.cmp(&left.workout_count))
            .then_with(|| left.name.cmp(&right.name))
    });
    ranked
        .into_iter()
        .take(2)
        .enumerate()
        .map(|(index, (item, score))| {
            let label = if index == 0 {
                "Best fit"
            } else {
                "Alternative"
            };
            let reason = if score > 0 {
                "Matches your training gaps."
            } else {
                "Fits this movement."
            };
            present_suggestion(
                item,
                if index == 0 { "expand" } else { "deepen" },
                label,
                reason.to_string(),
                score as f64,
                guide,
            )
        })
        .collect()
}

struct Scored<'a> {
    item: &'a ExerciseGuide,
    fit_score: i64,
    fatigue_penalty: f64,
    strongest_needed_muscle: String,
}

fn next_suggestions(
    draft: &Draft,
    guide: &GuideConfig,
    context: &SessionContext,
) -> (Option<Suggestion>, Option<Suggestion>) {
    let selected: HashSet<&str> = draft
        .exercises
        .iter()
        .map(|exercise| exercise.name.as_str())
        .collect();
    let deltas = remaining_deltas(&guide.muscle_needs, &context.muscle_load);
    // The session defines which needs Deepen can reward, not how strongly it
    // rewards them. Weighting by prior stimulus would favor the constituents
    // already getting the most work. Keep every surplus penalty in both lanes.
    let scoped_deltas: MuscleDeltas = deltas
        .iter()
        .filter(|(muscle, delta)| **delta < 0 || context.muscles.contains(*muscle))
        .map(|(muscle, delta)| (muscle.clone(), *delta))
        .collect();
    let candidates: Vec<_> = guide
        .exercises
        .iter()
        .filter(|item| !selected.contains(item.name.as_str()))
        .filter(|item| !item.muscles.is_empty() && !item.movements.iter().any(|m| m == "cardio"))
        .collect();
    let best = |deltas: &MuscleDeltas, exclude: Option<&str>| {
        candidates
            .iter()
            .filter(|item| Some(item.name.as_str()) != exclude)
            .map(|item| score_candidate(item, context, deltas))
            .filter(|scored| scored.fit_score > 0)
            .min_by(compare_scored)
    };
    // Always show the overall best fit. The scoped lane offers the best
    // distinct alternative, so the two buttons never duplicate an exercise.
    let expand = best(&deltas, None);
    let deep = best(
        &scoped_deltas,
        expand.as_ref().map(|scored| scored.item.name.as_str()),
    );
    (
        deep.map(|scored| scored_suggestion("deepen", &scored, guide)),
        expand.map(|scored| scored_suggestion("expand", &scored, guide)),
    )
}

fn compare_scored(left: &Scored<'_>, right: &Scored<'_>) -> Ordering {
    right
        .fit_score
        .cmp(&left.fit_score)
        .then_with(|| left.fatigue_penalty.total_cmp(&right.fatigue_penalty))
        .then_with(|| right.item.workout_count.cmp(&left.item.workout_count))
        .then_with(|| left.item.name.cmp(&right.item.name))
}

fn score_candidate<'a>(
    item: &'a ExerciseGuide,
    context: &SessionContext,
    deltas: &MuscleDeltas,
) -> Scored<'a> {
    let mut strongest_needed_muscle = String::new();
    let mut strongest_need = 0_i64;
    for (muscle, ratio) in &item.muscles {
        let contribution = deltas
            .get(muscle)
            .copied()
            .unwrap_or(0)
            .saturating_mul(i64::from(*ratio));
        if contribution > strongest_need {
            strongest_need = contribution;
            strongest_needed_muscle = muscle.clone();
        }
    }
    let shared_fatigue_movements = item
        .movements
        .iter()
        .filter(|movement| context.high_fatigue_movements.contains(*movement))
        .count();
    let shared_fatigue_regions = item
        .coarse_muscles
        .iter()
        .filter(|group| context.high_fatigue_coarse.contains(*group))
        .count();
    let fatigue_penalty = if item.high_fatigue && context.high_fatigue_count > 0 {
        shared_fatigue_movements as f64 * 150.0 + shared_fatigue_regions as f64 * 90.0
    } else {
        0.0
    };
    let axial_penalty = if item.high_axial_load && context.high_axial_count > 0 {
        600.0
    } else {
        0.0
    };
    Scored {
        item,
        fit_score: dot_product(deltas, &item.muscles),
        fatigue_penalty: fatigue_penalty + axial_penalty,
        strongest_needed_muscle,
    }
}

fn scored_suggestion(lane: &str, scored: &Scored<'_>, guide: &GuideConfig) -> Suggestion {
    let reason = if lane == "deepen" {
        format!(
            "Rounds out {}.",
            muscle_label(&scored.strongest_needed_muscle)
        )
    } else {
        format!(
            "Covers remaining {} need.",
            muscle_label(&scored.strongest_needed_muscle)
        )
    };
    present_suggestion(
        scored.item,
        lane,
        lane,
        reason,
        scored.fit_score as f64,
        guide,
    )
}

fn present_suggestion(
    item: &ExerciseGuide,
    lane: &str,
    label: &str,
    reason: String,
    score: f64,
    guide: &GuideConfig,
) -> Suggestion {
    let mark = item
        .marks
        .first()
        .map(|mark| format!("{} {}", mark.kind, mark.value))
        .unwrap_or_else(|| history_line(item, &guide.today));
    Suggestion {
        name: item.name.clone(),
        lane: lane.to_string(),
        label: label.to_string(),
        reason: reason.clone(),
        mark: mark.clone(),
        aria_label: format!("Add {} — {} — {}", item.name, reason, mark),
        score,
    }
}

fn search(draft: &Draft, guide: &GuideConfig, query: &str) -> (Vec<SearchHit>, String) {
    let query = query.to_lowercase();
    let terms: Vec<&str> = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .collect();
    if terms.is_empty() {
        return (Vec::new(), String::new());
    }
    let selected: HashSet<&str> = draft
        .exercises
        .iter()
        .map(|exercise| exercise.name.as_str())
        .collect();
    let matches: Vec<_> = search_exercises(&guide.exercises, &query)
        .into_iter()
        .filter(|item| !selected.contains(item.name.as_str()))
        .collect();
    let total = matches.len();
    let hits: Vec<SearchHit> = matches
        .into_iter()
        .take(SEARCH_LIMIT)
        .map(|item| SearchHit {
            name: item.name.clone(),
        })
        .collect();
    let shown = hits.len();
    let feedback = if shown == 0 {
        "No matching exercises.".to_string()
    } else if total > shown {
        format!("{shown} of {total} matching exercises shown.")
    } else {
        format!(
            "{shown} matching {}.",
            if shown == 1 { "exercise" } else { "exercises" }
        )
    };
    (hits, feedback)
}

/// Ranked catalog search shared by entry and the standalone library.
pub fn search_exercises<'a>(exercises: &'a [ExerciseGuide], query: &str) -> Vec<&'a ExerciseGuide> {
    let query = query.trim().to_lowercase();
    let terms: Vec<&str> = query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .collect();
    let mut matches: Vec<_> = exercises
        .iter()
        .filter(|item| matches_search_terms(&search_text(item), &terms))
        .collect();
    matches.sort_by(|left, right| {
        search_rank(left, &query, &terms)
            .cmp(&search_rank(right, &query, &terms))
            .then_with(|| right.workout_count.cmp(&left.workout_count))
            .then_with(|| left.name.cmp(&right.name))
    });
    matches
}

fn search_text(item: &ExerciseGuide) -> String {
    let muscles = item.muscles.iter().map(|(muscle, _)| muscle.as_str());
    std::iter::once(item.name.as_str())
        .chain(item.aliases.iter().map(String::as_str))
        .chain(item.equipment.iter().map(String::as_str))
        .chain(item.movements.iter().map(String::as_str))
        .chain(item.coarse_muscles.iter().map(String::as_str))
        .chain(muscles)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn matches_search_terms(text: &str, terms: &[&str]) -> bool {
    // Each term must match, but words may be separated or reordered. Substring
    // matching preserves partial input such as "bicep" matching "biceps".
    terms.iter().all(|term| text.contains(term))
}

fn search_rank(item: &ExerciseGuide, query: &str, terms: &[&str]) -> u8 {
    std::iter::once(&item.name)
        .chain(item.aliases.iter())
        .map(|name| {
            let name = name.to_lowercase();
            if name == query {
                0
            } else if name.starts_with(query) {
                1
            } else if name.split_whitespace().any(|word| word.starts_with(query)) {
                2
            } else if name.contains(query) {
                3
            } else if matches_search_terms(&name, terms) {
                4
            } else {
                5
            }
        })
        .min()
        .unwrap_or(5)
}

fn history_line(item: &ExerciseGuide, today: &str) -> String {
    let workouts = format!(
        "{} {}",
        item.workout_count,
        if item.workout_count == 1 {
            "workout"
        } else {
            "workouts"
        }
    );
    let last = if item.last_date.is_empty() {
        "no dated history".to_string()
    } else {
        format!("last {}", relative_date(today, &item.last_date))
    };
    format!("{workouts} · {last}")
}

fn relative_date(today: &str, date: &str) -> String {
    match days_since(today, date) {
        Some(days) if days <= 0 => "today".to_string(),
        Some(1) => "yesterday".to_string(),
        Some(days) if days < 14 => format!("{days}d ago"),
        Some(days) if days < 56 => format!("{}w ago", (days as f64 / 7.0).round()),
        _ => date.to_string(),
    }
}

fn days_since(today: &str, date: &str) -> Option<i64> {
    let end = eastern_time::utc_timestamp(&format!("{today} 12:00:00")).ok()?;
    let start = eastern_time::utc_timestamp(&format!("{date} 12:00:00")).ok()?;
    Some((end.as_second() - start.as_second()) / 86_400)
}

fn muscle_label(id: &str) -> String {
    id.replace("glute-max", "glute max")
        .replace("glute-med", "glute med")
        .replace('-', " ")
}

fn load_display(weight_milli: Option<i64>, bodyweight: bool) -> String {
    let Some(weight) = weight_milli else {
        return if bodyweight { "BW" } else { "—" }.to_string();
    };
    let amount = crate::text::weight_text(weight);
    if bodyweight && weight > 0 {
        format!("+{amount} lb")
    } else if weight < 0 {
        format!("−{} lb", amount.trim_start_matches('-'))
    } else {
        format!("{amount} lb")
    }
}

fn load_spoken(weight_milli: Option<i64>, bodyweight: bool) -> String {
    let Some(weight) = weight_milli else {
        return if bodyweight { "bodyweight" } else { "no load" }.to_string();
    };
    let amount = crate::text::weight_text(weight.abs());
    if weight < 0 {
        format!("{amount} pounds assistance")
    } else if bodyweight && weight > 0 {
        format!("{amount} pounds added")
    } else {
        format!("{amount} pounds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::{DraftExercise, DraftSet};

    #[test]
    fn library_and_entry_search_find_aliases_and_equipment() {
        let mut item = ExerciseGuide::fixture("Canonical Curl");
        item.aliases = vec!["Old Hammer Curl".into()];
        item.equipment = vec!["dumbbell".into()];
        let catalog = vec![item];
        assert_eq!(
            search_exercises(&catalog, "hammer old")[0].name,
            "Canonical Curl"
        );
        assert_eq!(
            search_exercises(&catalog, "dumbbell curl")[0].name,
            "Canonical Curl"
        );
        assert!(search_exercises(&catalog, "unrelated").is_empty());
    }

    fn item(
        name: &str,
        workouts: usize,
        last: &str,
        muscles: &[(&str, u32)],
        movements: &[&str],
        coarse: &[&str],
    ) -> ExerciseGuide {
        ExerciseGuide {
            aliases: Vec::new(),
            equipment: Vec::new(),
            name: name.into(),
            bodyweight: false,
            high_fatigue: false,
            high_axial_load: false,
            last_date: last.into(),
            set_count: workouts * 4,
            workout_count: workouts,
            muscles: muscles
                .iter()
                .map(|(name, ratio)| ((*name).into(), *ratio))
                .collect(),
            movements: movements.iter().map(|value| (*value).into()).collect(),
            coarse_muscles: coarse.iter().map(|value| (*value).into()).collect(),
            marks: Vec::new(),
            loads: Vec::new(),
            picker_meta: String::new(),
            picker_mark: String::new(),
        }
    }

    fn guide() -> GuideConfig {
        GuideConfig {
            version: 3,
            today: "2026-09-03".into(),
            weekly_pace_tenths: 20,
            muscle_needs: BTreeMap::from([("triceps".into(), 8000), ("hamstrings".into(), 4800)]),
            exercises: vec![
                item(
                    "Bench Press",
                    20,
                    "2026-09-01",
                    &[("chest", 100), ("triceps", 50)],
                    &["horizontal-push"],
                    &["chest"],
                ),
                item(
                    "Triceps Extension",
                    5,
                    "2026-08-01",
                    &[("triceps", 100)],
                    &["elbow-extension"],
                    &["arms"],
                ),
                item(
                    "Squat",
                    15,
                    "2026-08-30",
                    &[("quads", 100), ("hamstrings", 40)],
                    &["squat-type"],
                    &["legs"],
                ),
                item(
                    "Leg Curl",
                    4,
                    "2026-07-01",
                    &[("hamstrings", 100)],
                    &["knee-flexion"],
                    &["legs"],
                ),
                item(
                    "Incline Press",
                    8,
                    "2026-08-20",
                    &[("chest", 90), ("triceps", 55)],
                    &["horizontal-push"],
                    &["chest"],
                ),
            ],
        }
    }

    fn draft_with(name: &str) -> Draft {
        Draft {
            version: 1,
            started_at_utc: "2026-09-03 14:00:00".into(),
            title: "Workout".into(),
            notes: String::new(),
            exercises: vec![DraftExercise {
                id: "exercise-0001".into(),
                name: name.into(),
                sets: vec![DraftSet {
                    id: "set-00000001".into(),
                    weight: "100".into(),
                    reps: "5".into(),
                    effort: "9".into(),
                    failure: false,
                    set_type: SetType::Normal,
                    done: true,
                }],
            }],
        }
    }

    #[test]
    fn volume_points_match_the_archive_scale() {
        assert_eq!(set_volume_points(SetType::Warmup, Some(1_000), true), 0);
        assert_eq!(set_volume_points(SetType::Normal, None, true), 6);
        assert_eq!(set_volume_points(SetType::Normal, Some(1_000), false), 5);
        assert_eq!(set_volume_points(SetType::Normal, Some(900), false), 4);
        assert_eq!(set_volume_points(SetType::Normal, Some(800), false), 3);
        assert_eq!(set_volume_points(SetType::Normal, Some(750), false), 2);
    }

    #[test]
    fn overall_and_scoped_recommendations_cover_different_needs() {
        let mut guide = guide();
        guide.muscle_needs.insert("hamstrings".into(), 20_000);
        let derived = derive(
            &draft_with("Bench Press"),
            &guide,
            &GuidanceContext::default(),
        );
        assert_eq!(derived.coverage[0].muscle, "chest");
        assert_eq!(derived.coverage[0].level, "main");
        assert_eq!(derived.deepen.as_ref().unwrap().name, "Triceps Extension");
        assert_eq!(
            derived.deepen.as_ref().unwrap().reason,
            "Rounds out triceps."
        );
        assert_eq!(derived.expand.as_ref().unwrap().name, "Leg Curl");
    }

    #[test]
    fn the_overall_winner_is_preserved_when_both_lanes_prefer_it() {
        let derived = derive(
            &draft_with("Bench Press"),
            &guide(),
            &GuidanceContext::default(),
        );
        assert_eq!(derived.expand.as_ref().unwrap().name, "Triceps Extension");
        assert_eq!(derived.deepen.as_ref().unwrap().name, "Incline Press");
    }

    #[test]
    fn dips_lead_to_lagging_triceps_or_a_broader_lower_body_fit() {
        let mut guide = guide();
        guide.muscle_needs = MuscleDeltas::from([
            ("lower-chest".into(), 8_000),
            ("triceps".into(), 12_800),
            ("glute-max".into(), 16_000),
            ("hamstrings".into(), 16_000),
            ("lats".into(), 20_000),
        ]);
        guide.exercises = vec![
            item(
                "Dip",
                20,
                "2026-09-01",
                &[("lower-chest", 100), ("triceps", 50)],
                &["dip"],
                &["chest"],
            ),
            item(
                "Deadlift",
                20,
                "2026-09-01",
                &[("glute-max", 100), ("hamstrings", 100)],
                &["hinge"],
                &["legs"],
            ),
            item(
                "Triceps Extension",
                5,
                "2026-09-01",
                &[("triceps", 100)],
                &["elbow-extension"],
                &["arms"],
            ),
            item(
                "Chest Fly",
                20,
                "2026-09-01",
                &[("lower-chest", 100)],
                &["horizontal-push"],
                &["chest"],
            ),
            item(
                "Pullover",
                20,
                "2026-09-01",
                &[("lats", 100), ("triceps", 5)],
                &["shoulder-extension"],
                &["back"],
            ),
        ];
        let mut draft = draft_with("Dip");
        let mut second = draft.exercises[0].sets[0].clone();
        second.id = "set-00000002".into();
        draft.exercises[0].sets.push(second);
        for _ in 0..2 {
            let derived = derive(&draft, &guide, &GuidanceContext::default());
            assert_eq!(derived.expand.as_ref().unwrap().name, "Deadlift");
            assert_eq!(derived.expand.as_ref().unwrap().score, 3_200_000.0);
            assert_eq!(derived.deepen.as_ref().unwrap().name, "Triceps Extension");
            assert_eq!(derived.deepen.as_ref().unwrap().score, 960_000.0);
            // Planned work has the same dose as completed work. Neither
            // completing rows nor catalog order should change these results.
            for set in &mut draft.exercises[0].sets {
                set.done = false;
            }
            guide.exercises.reverse();
        }
        // Unrelated needs cannot earn scoped credit, but unrelated surpluses
        // still penalize an otherwise strong triceps candidate.
        guide.muscle_needs.insert("lats".into(), -20_000);
        let extension = guide
            .exercises
            .iter_mut()
            .find(|item| item.name == "Triceps Extension")
            .unwrap();
        extension.muscles.push(("lats".into(), 100));
        let derived = derive(&draft, &guide, &GuidanceContext::default());
        assert_eq!(derived.deepen.as_ref().unwrap().name, "Chest Fly");
        assert_eq!(derived.deepen.as_ref().unwrap().score, 160_000.0);
    }

    #[test]
    fn starter_and_search_ordering_are_deterministic() {
        let empty = Draft {
            version: 1,
            started_at_utc: "2026-09-03 14:00:00".into(),
            title: "Workout".into(),
            notes: String::new(),
            exercises: Vec::new(),
        };
        let starters = derive(
            &empty,
            &guide(),
            &GuidanceContext {
                direction: "push".into(),
                query: String::new(),
            },
        );
        assert_eq!(starters.starters[0].name, "Incline Press");
        assert_eq!(starters.starters[0].label, "Best fit");

        let search = derive(
            &empty,
            &guide(),
            &GuidanceContext {
                direction: String::new(),
                query: "press".into(),
            },
        );
        assert_eq!(
            search
                .search
                .iter()
                .map(|hit| hit.name.as_str())
                .collect::<Vec<_>>(),
            ["Bench Press", "Incline Press"]
        );
    }

    #[test]
    fn starters_and_next_picks_share_compound_and_surplus_ranking() {
        let mut guide = guide();
        guide.muscle_needs = MuscleDeltas::from([("chest".into(), 8000), ("triceps".into(), 8000)]);
        guide
            .exercises
            .retain(|item| ["Bench Press", "Leg Curl"].contains(&item.name.as_str()));
        guide.exercises.push(item(
            "Chest Fly",
            100,
            "2026-07-01",
            &[("chest", 100)],
            &["horizontal-push"],
            &["chest"],
        ));
        let mut empty = draft_with("Leg Curl");
        empty.exercises.clear();
        let direction = GuidanceContext {
            direction: "push".into(),
            query: String::new(),
        };
        for (triceps, expected) in [(8000, "Bench Press"), (-8000, "Chest Fly")] {
            guide.muscle_needs.insert("triceps".into(), triceps);
            for _ in 0..2 {
                let starters = derive(&empty, &guide, &direction);
                let next = derive(&draft_with("Leg Curl"), &guide, &GuidanceContext::default());
                assert_eq!(starters.starters[0].name, expected);
                assert_eq!(next.expand.as_ref().unwrap().name, expected);
                assert_eq!(starters.starters[0].score, next.expand.unwrap().score);
                guide.exercises.reverse();
            }
        }
    }

    #[test]
    fn planned_sets_shift_the_best_fit_gradually_and_warmups_do_not() {
        let mut guide = guide();
        guide.muscle_needs = MuscleDeltas::from([("chest".into(), 6000), ("triceps".into(), 4000)]);
        guide
            .exercises
            .retain(|item| ["Bench Press", "Triceps Extension"].contains(&item.name.as_str()));
        guide.exercises.push(item(
            "Chest Fly",
            5,
            "2026-08-01",
            &[("chest", 100)],
            &["horizontal-push"],
            &["chest"],
        ));
        let mut draft = draft_with("Bench Press");
        draft.exercises[0].sets[0].done = false;
        draft.exercises[0].sets[0].reps.clear();
        let first = derive(&draft, &guide, &GuidanceContext::default());
        assert_eq!(first.expand.as_ref().unwrap().name, "Chest Fly");
        assert_eq!(first.expand.as_ref().unwrap().score, 280_000.0);
        assert_eq!(
            first.deepen.as_ref().unwrap().score,
            240_000.0,
            "partial triceps credit leaves a real gap"
        );

        let mut second = draft.exercises[0].sets[0].clone();
        second.id = "set-00000002".into();
        second.set_type = SetType::Warmup;
        draft.exercises[0].sets.push(second);
        let warm = derive(&draft, &guide, &GuidanceContext::default());
        assert_eq!(warm.deepen, first.deepen);
        assert_eq!(warm.expand, first.expand);

        draft.exercises[0].sets[1].set_type = SetType::Normal;
        let second = derive(&draft, &guide, &GuidanceContext::default());
        assert_eq!(second.expand.as_ref().unwrap().name, "Triceps Extension");
        assert_eq!(second.expand.as_ref().unwrap().score, 80_000.0);
        assert!(
            second.deepen.is_none(),
            "the remaining chest option is above target"
        );

        let mut third = draft.exercises[0].sets[0].clone();
        third.id = "set-00000003".into();
        draft.exercises[0].sets.push(third);
        let met = derive(&draft, &guide, &GuidanceContext::default());
        assert!(met.expand.is_none());
        assert!(met.deepen.is_none());
        draft.exercises[0].sets.pop();

        draft.exercises[0].sets.pop();
        let restored = derive(&draft, &guide, &GuidanceContext::default());
        assert_eq!(restored.deepen, first.deepen);
        assert_eq!(restored.expand, first.expand);
    }

    #[test]
    fn exercise_search_matches_separated_reordered_and_partial_words() {
        let mut guide = guide();
        guide.exercises = vec![
            item("Barbell Biceps Curl", 5, "2026-09-01", &[], &[], &[]),
            item("Dumbbell Biceps Curl", 20, "2026-09-01", &[], &[], &[]),
            item("Barbell Bench Press", 30, "2026-09-01", &[], &[], &[]),
        ];
        let mut draft = draft_with("Bench Press");
        draft.exercises.clear();
        for query in [
            "barbell curl",
            "curl barbell",
            "BARBELL  CURL",
            "barbell-curl",
        ] {
            let (hits, _) = search(&draft, &guide, query);
            assert_eq!(hits.len(), 1, "query: {query}");
            assert_eq!(hits[0].name, "Barbell Biceps Curl", "query: {query}");
        }
        let (hits, _) = search(&draft, &guide, "curl bicep");
        assert_eq!(
            hits.iter().map(|hit| hit.name.as_str()).collect::<Vec<_>>(),
            ["Dumbbell Biceps Curl", "Barbell Biceps Curl"]
        );
        assert!(search(&draft, &guide, "barbell squat").0.is_empty());
        assert!(search(&draft, &guide, " -- ").0.is_empty());
        assert!(search(&draft, &guide, "  ").0.is_empty());
    }

    #[test]
    fn exercise_search_prefers_names_over_metadata_and_excludes_selected() {
        let mut guide = guide();
        guide.exercises = vec![
            item("Barbell Biceps Curl", 5, "2026-09-01", &[], &[], &[]),
            item("Bicep Curl", 1, "2026-09-01", &[], &[], &[]),
            item("Cable Curl", 50, "2026-09-01", &[("biceps", 100)], &[], &[]),
        ];
        let mut draft = draft_with("Bench Press");
        draft.exercises.clear();
        let (hits, _) = search(&draft, &guide, "bicep curl");
        assert_eq!(
            hits.iter().map(|hit| hit.name.as_str()).collect::<Vec<_>>(),
            ["Bicep Curl", "Barbell Biceps Curl", "Cable Curl"]
        );
        let (hits, _) = search(&draft_with("Bicep Curl"), &guide, "curl bicep");
        assert_eq!(
            hits.iter().map(|hit| hit.name.as_str()).collect::<Vec<_>>(),
            ["Barbell Biceps Curl", "Cable Curl"]
        );
    }

    #[test]
    fn recommendations_open_for_planned_exercises_and_recompute_for_added_rows() {
        let mut draft = draft_with("Bench Press");
        draft.exercises[0].sets[0].done = false;
        draft.exercises[0].sets[0].reps.clear();

        let one_row = derive(&draft, &guide(), &GuidanceContext::default());
        assert!(one_row.has_active_exercise);
        assert!(!one_row.has_completed_set);
        assert_eq!(one_row.expand.as_ref().unwrap().name, "Triceps Extension");

        let mut second = draft.exercises[0].sets[0].clone();
        second.id = "set-00000002".into();
        draft.exercises[0].sets.push(second);
        let two_rows = derive(&draft, &guide(), &GuidanceContext::default());
        assert!(
            two_rows.expand.as_ref().unwrap().score < one_row.expand.as_ref().unwrap().score,
            "another planned set reduces the remaining gap before completion"
        );
    }

    #[test]
    fn fatigue_breaks_equal_fits_but_does_not_override_greater_need() {
        let mut guide = GuideConfig {
            version: 1,
            today: "2026-09-03".into(),
            weekly_pace_tenths: 20,
            muscle_needs: BTreeMap::from([("spinal-erectors".into(), 1_000)]),
            exercises: vec![
                item(
                    "Full Squat",
                    20,
                    "2026-09-01",
                    &[("quads", 100), ("spinal-erectors", 25)],
                    &["squat-type"],
                    &["legs"],
                ),
                item(
                    "Sumo Deadlift",
                    30,
                    "2026-09-01",
                    &[("spinal-erectors", 100), ("hamstrings", 75)],
                    &["hinge"],
                    &["legs", "back"],
                ),
                item(
                    "Back Extension",
                    5,
                    "2026-08-01",
                    &[("spinal-erectors", 100)],
                    &["hinge"],
                    &["back"],
                ),
                item(
                    "Leg Curl",
                    4,
                    "2026-08-01",
                    &[("hamstrings", 100)],
                    &["knee-flexion"],
                    &["legs"],
                ),
            ],
        };
        for name in ["Full Squat", "Sumo Deadlift"] {
            let exercise = guide
                .exercises
                .iter_mut()
                .find(|exercise| exercise.name == name)
                .unwrap();
            exercise.high_fatigue = true;
            exercise.high_axial_load = true;
        }
        let mut draft = draft_with("Full Squat");
        draft.exercises[0].sets[0].done = false;

        let derived = derive(&draft, &guide, &GuidanceContext::default());
        assert_eq!(derived.expand.as_ref().unwrap().name, "Back Extension");

        guide.muscle_needs.insert("hamstrings".into(), 8_000);
        guide.muscle_needs.insert("spinal-erectors".into(), 10_000);
        let derived = derive(&draft, &guide, &GuidanceContext::default());
        assert_eq!(derived.expand.as_ref().unwrap().name, "Sumo Deadlift");
        assert_eq!(derived.deepen.as_ref().unwrap().name, "Back Extension");
    }

    #[test]
    fn load_presentations_keep_null_zero_and_assistance_distinct() {
        assert_eq!(
            LoadPreset::new("work", None, SetType::Normal, true).display,
            "BW"
        );
        assert_eq!(
            LoadPreset::new("work", Some(0), SetType::Normal, true).display,
            "0 lb"
        );
        assert_eq!(
            LoadPreset::new("work", Some(-40_000), SetType::Normal, true).display,
            "−40 lb"
        );
    }
}
