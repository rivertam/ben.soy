use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::MAX_WEIGHT_MILLI;
use crate::draft::{ActionError, Draft, DraftSet};
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
    pub muscle_needs: BTreeMap<String, u32>,
    pub exercises: Vec<ExerciseGuide>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExerciseGuide {
    pub name: String,
    pub bodyweight: bool,
    /// Coarse fatigue metadata used only to avoid redundant recommendations.
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
    pub(crate) fn validate(&self) -> Result<(), ActionError> {
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
    let (rir_display, rir_spoken) = match (set.failure, effort_blank, effort_hundredths) {
        (true, true, _) => ("FAIL".to_string(), "Reached failure".to_string()),
        (true, false, _) => ("?".to_string(), "Invalid failure effort".to_string()),
        (false, true, _) => ("—".to_string(), "Not rated".to_string()),
        (false, false, Some(effort)) => {
            let rir = 1_000 - effort;
            (hundredths_text(rir), rir_spoken(effort))
        }
        (false, false, None) => ("?".to_string(), "Invalid reps in reserve".to_string()),
    };
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
    primary: BTreeSet<String>,
    secondary: BTreeSet<String>,
    movements: BTreeSet<String>,
    coarse: BTreeSet<String>,
    exercise_count: usize,
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
        context.exercise_count += 1;
        context.movements.extend(item.movements.iter().cloned());
        context.coarse.extend(item.coarse_muscles.iter().cloned());
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
            if *ratio >= 75 {
                context.primary.insert(muscle.clone());
            } else {
                context.secondary.insert(muscle.clone());
            }
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
    let max_need = guide
        .muscle_needs
        .values()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let needed = best_by(&candidates, |item| {
        starter_score(item, guide, max_need, false)
    });
    let familiar_candidates: Vec<&ExerciseGuide> = candidates
        .iter()
        .copied()
        .filter(|item| Some(item.name.as_str()) != needed.map(|item| item.name.as_str()))
        .collect();
    let familiar = best_by(&familiar_candidates, |item| {
        starter_score(item, guide, max_need, true)
    });
    let needed_label = needed
        .filter(|item| starter_need(item, guide, max_need) > 0.0)
        .map_or("Less recent", |_| "Behind pace");
    let mut suggestions = Vec::new();
    if let Some(item) = familiar.or(needed) {
        let (label, lane, score) = if familiar.is_some() {
            (
                "Familiar",
                "deepen",
                starter_score(item, guide, max_need, true),
            )
        } else {
            (
                needed_label,
                "expand",
                starter_score(item, guide, max_need, false),
            )
        };
        suggestions.push(present_suggestion(
            item,
            lane,
            label,
            label.to_string(),
            score,
            guide,
        ));
    }
    if familiar.is_some()
        && let Some(item) = needed
    {
        suggestions.push(present_suggestion(
            item,
            "expand",
            needed_label,
            needed_label.to_string(),
            starter_score(item, guide, max_need, false),
            guide,
        ));
    }
    suggestions
}

fn best_by<'a>(
    items: &'a [&'a ExerciseGuide],
    score: impl Fn(&ExerciseGuide) -> f64,
) -> Option<&'a ExerciseGuide> {
    items.iter().copied().max_by(|left, right| {
        score(left)
            .total_cmp(&score(right))
            .then_with(|| left.workout_count.cmp(&right.workout_count))
            .then_with(|| right.name.cmp(&left.name))
    })
}

fn starter_score(item: &ExerciseGuide, guide: &GuideConfig, max_need: f64, familiar: bool) -> f64 {
    let familiarity = ((item.workout_count + 1) as f64).log2();
    let days = days_since(&guide.today, &item.last_date).unwrap_or(30);
    let staleness = days.clamp(0, 30) as f64 / 30.0;
    let need = starter_need(item, guide, max_need);
    if familiar {
        familiarity * 30.0 + ((item.set_count + 1) as f64).log2() + need * 4.0
    } else {
        need * 90.0 + staleness * 25.0 + familiarity * 2.0
    }
}

fn starter_need(item: &ExerciseGuide, guide: &GuideConfig, max_need: f64) -> f64 {
    item.muscles
        .iter()
        .map(|(muscle, ratio)| {
            f64::from(*guide.muscle_needs.get(muscle).unwrap_or(&0)) / max_need
                * (f64::from(*ratio) / 100.0)
        })
        .sum()
}

#[derive(Clone)]
struct Scored<'a> {
    item: &'a ExerciseGuide,
    deep_score: f64,
    expand_score: f64,
    complement: f64,
    strongest_needed_muscle: String,
    novel: usize,
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
    let max_need = guide
        .muscle_needs
        .values()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1) as f64;
    let weekly_pace = guide.weekly_pace_tenths as f64 / 10.0;
    let breadth = (1.0 + (3.5 - weekly_pace).max(0.0) / 2.0).min(2.25);
    let mut scored: Vec<Scored<'_>> = guide
        .exercises
        .iter()
        .filter(|item| !selected.contains(item.name.as_str()))
        .filter(|item| !item.muscles.is_empty() && !item.movements.iter().any(|m| m == "cardio"))
        .map(|item| score_candidate(item, guide, context, max_need, breadth))
        .collect();
    scored.sort_by(compare_scored(|item| item.deep_score));
    let deep = scored.first().cloned();
    scored.sort_by(compare_scored(|item| item.expand_score));
    let expand = scored.into_iter().find(|candidate| {
        deep.as_ref()
            .is_none_or(|deep| candidate.item.name != deep.item.name)
    });
    (
        deep.map(|scored| scored_suggestion("deepen", &scored, context, guide)),
        expand.map(|scored| scored_suggestion("expand", &scored, context, guide)),
    )
}

fn compare_scored(
    score: impl Fn(&Scored<'_>) -> f64 + Copy,
) -> impl Fn(&Scored<'_>, &Scored<'_>) -> Ordering {
    move |left, right| {
        score(right)
            .total_cmp(&score(left))
            .then_with(|| right.item.workout_count.cmp(&left.item.workout_count))
            .then_with(|| left.item.name.cmp(&right.item.name))
    }
}

fn score_candidate<'a>(
    item: &'a ExerciseGuide,
    guide: &GuideConfig,
    context: &SessionContext,
    max_need: f64,
    breadth: f64,
) -> Scored<'a> {
    let mut overlap = 0.0;
    let mut bridge = 0;
    let mut need = 0.0;
    let mut strongest_needed_muscle = String::new();
    let mut strongest_need = 0.0;
    for (muscle, ratio) in &item.muscles {
        let session = f64::from(*context.muscle_load.get(muscle).unwrap_or(&0));
        overlap += session * (f64::from(*ratio) / 100.0);
        let muscle_need = if session > 0.0 {
            0.0
        } else {
            f64::from(*guide.muscle_needs.get(muscle).unwrap_or(&0)) / max_need
        };
        need += muscle_need * (f64::from(*ratio) / 100.0);
        if muscle_need * f64::from(*ratio) > strongest_need {
            strongest_need = muscle_need * f64::from(*ratio);
            strongest_needed_muscle = muscle.clone();
        }
        if *ratio >= 75 && context.secondary.contains(muscle) && !context.primary.contains(muscle) {
            bridge += 1;
        }
    }
    let novel = item
        .coarse_muscles
        .iter()
        .filter(|group| !context.coarse.contains(*group))
        .count();
    let movement_overlap = item
        .movements
        .iter()
        .filter(|movement| context.movements.contains(*movement))
        .count();
    let complement = complement_score(item, context);
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
    let staleness = days_since(&guide.today, &item.last_date)
        .unwrap_or(30)
        .min(30) as f64
        / 30.0;
    let familiarity = ((item.workout_count + 1) as f64).log2();
    let deep_score = if context.exercise_count == 0 {
        familiarity * 16.0 + need * 28.0 + staleness * 8.0
    } else {
        overlap * 0.16
            + f64::from(bridge) * 72.0
            + movement_overlap as f64 * 28.0
            + complement * 80.0
            + need * 18.0
            + familiarity
            - fatigue_penalty
            - axial_penalty
    };
    let expand_score = need * 96.0 * breadth
        + novel as f64 * 34.0 * breadth
        + staleness * 15.0
        + familiarity * 1.5
        - overlap * 0.08
        - fatigue_penalty
        - axial_penalty;
    Scored {
        item,
        deep_score,
        expand_score,
        complement,
        strongest_needed_muscle,
        novel,
    }
}

fn complement_score(item: &ExerciseGuide, context: &SessionContext) -> f64 {
    let has = |movement: &str| context.movements.contains(movement);
    let candidate_has = |movement: &str| item.movements.iter().any(|value| value == movement);
    let needs_primary_complement = ((has("horizontal-push") || has("vertical-push") || has("dip"))
        && candidate_has("elbow-extension")
        && !has("elbow-extension"))
        || ((has("horizontal-pull") || has("vertical-pull"))
            && candidate_has("elbow-flexion")
            && !has("elbow-flexion"))
        || ((has("squat-type") || has("hinge"))
            && candidate_has("knee-flexion")
            && !has("knee-flexion"));
    if needs_primary_complement {
        1.0
    } else if has("squat-type") && candidate_has("calf-raise") && !has("calf-raise") {
        0.8
    } else if (has("horizontal-push") || has("vertical-push"))
        && candidate_has("shoulder-abduction")
        && !has("shoulder-abduction")
    {
        0.7
    } else {
        0.0
    }
}

fn scored_suggestion(
    lane: &str,
    scored: &Scored<'_>,
    context: &SessionContext,
    guide: &GuideConfig,
) -> Suggestion {
    let reason = if lane == "deepen" {
        if scored.complement > 0.0 {
            if scored
                .item
                .movements
                .iter()
                .any(|value| value == "elbow-extension")
            {
                "Direct triceps.".to_string()
            } else if scored
                .item
                .movements
                .iter()
                .any(|value| value == "elbow-flexion")
            {
                "Direct elbow flexors.".to_string()
            } else if scored
                .item
                .movements
                .iter()
                .any(|value| value == "knee-flexion")
            {
                "Direct hamstrings.".to_string()
            } else if scored
                .item
                .movements
                .iter()
                .any(|value| value == "calf-raise")
            {
                "Direct calves.".to_string()
            } else {
                "Adds the missing isolation.".to_string()
            }
        } else if let Some(shared) = scored
            .item
            .muscles
            .iter()
            .map(|(muscle, _)| muscle)
            .find(|muscle| context.muscle_load.get(*muscle).copied().unwrap_or(0) > 0)
        {
            format!("{} again.", muscle_label(shared))
        } else {
            "Same movement pattern.".to_string()
        }
    } else if !scored.strongest_needed_muscle.is_empty() {
        format!(
            "{} is behind pace.",
            muscle_label(&scored.strongest_needed_muscle)
        )
    } else if scored.novel > 0 {
        "Adds a new region.".to_string()
    } else {
        "Changes the pattern.".to_string()
    };
    let score = if lane == "deepen" {
        scored.deep_score
    } else {
        scored.expand_score
    };
    present_suggestion(scored.item, lane, lane, reason, score, guide)
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
    if query.is_empty() {
        return (Vec::new(), String::new());
    }
    let selected: HashSet<&str> = draft
        .exercises
        .iter()
        .map(|exercise| exercise.name.as_str())
        .collect();
    let mut matches: Vec<&ExerciseGuide> = guide
        .exercises
        .iter()
        .filter(|item| !selected.contains(item.name.as_str()))
        .filter(|item| search_text(item).contains(query))
        .collect();
    matches.sort_by(|left, right| {
        search_rank(left, query)
            .cmp(&search_rank(right, query))
            .then_with(|| right.workout_count.cmp(&left.workout_count))
            .then_with(|| left.name.cmp(&right.name))
    });
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

fn search_text(item: &ExerciseGuide) -> String {
    let muscles = item.muscles.iter().map(|(muscle, _)| muscle.as_str());
    std::iter::once(item.name.as_str())
        .chain(item.movements.iter().map(String::as_str))
        .chain(item.coarse_muscles.iter().map(String::as_str))
        .chain(muscles)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn search_rank(item: &ExerciseGuide, query: &str) -> u8 {
    let name = item.name.to_lowercase();
    if name == query {
        0
    } else if name.starts_with(query) {
        1
    } else if name.split_whitespace().any(|word| word.starts_with(query)) {
        2
    } else if name.contains(query) {
        3
    } else {
        4
    }
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

    fn item(
        name: &str,
        workouts: usize,
        last: &str,
        muscles: &[(&str, u32)],
        movements: &[&str],
        coarse: &[&str],
    ) -> ExerciseGuide {
        ExerciseGuide {
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
            muscle_needs: BTreeMap::from([("triceps".into(), 90), ("hamstrings".into(), 60)]),
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
    fn coverage_and_complement_recommendations_are_golden() {
        let derived = derive(
            &draft_with("Bench Press"),
            &guide(),
            &GuidanceContext::default(),
        );
        assert_eq!(derived.coverage[0].muscle, "chest");
        assert_eq!(derived.coverage[0].level, "main");
        assert_eq!(derived.deepen.as_ref().unwrap().name, "Triceps Extension");
        assert_eq!(derived.deepen.as_ref().unwrap().reason, "Direct triceps.");
        assert_eq!(derived.expand.as_ref().unwrap().name, "Leg Curl");
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
        assert_eq!(starters.starters[0].name, "Bench Press");
        assert_eq!(starters.starters[0].label, "Familiar");

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
    fn recommendations_open_for_planned_exercises_and_recompute_for_added_rows() {
        let mut draft = draft_with("Bench Press");
        draft.exercises[0].sets[0].done = false;
        draft.exercises[0].sets[0].reps.clear();

        let one_row = derive(&draft, &guide(), &GuidanceContext::default());
        assert!(one_row.has_active_exercise);
        assert!(!one_row.has_completed_set);
        assert_eq!(one_row.deepen.as_ref().unwrap().name, "Triceps Extension");

        let mut second = draft.exercises[0].sets[0].clone();
        second.id = "set-00000002".into();
        draft.exercises[0].sets.push(second);
        let two_rows = derive(&draft, &guide(), &GuidanceContext::default());
        assert!(
            two_rows.deepen.as_ref().unwrap().score > one_row.deepen.as_ref().unwrap().score,
            "another planned set should increase the session-overlap score"
        );
    }

    #[test]
    fn an_axial_compound_is_not_recommended_after_another_axial_compound() {
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
        assert_ne!(derived.deepen.as_ref().unwrap().name, "Sumo Deadlift");
        assert_ne!(derived.expand.as_ref().unwrap().name, "Sumo Deadlift");
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
