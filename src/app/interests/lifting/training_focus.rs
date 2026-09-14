//! Page-only lifting load and next-focus guidance for `/fitness`.
//!
//! This is intentionally an approximation, not a hypertrophy prescription.
//! It scales the archive's effort-weighted volume score by each exercise's
//! stored muscle ratios (`exercise_muscles`, in hundredths), then compares
//! the last seven Eastern dates with this archive's own pace over the eight
//! preceding weeks, or an explicit weekly target. Credit accumulates in exact integer centi-points
//! (points × ratio_hundredths); display rounds once, half away from zero.

use std::collections::{BTreeSet, HashMap};

pub(super) use fitness_entry_core::muscle_load::BASELINE_WEEKS;
use fitness_entry_core::muscle_load::{MuscleDeltas, delta_scaled, dot_product};
use jiff::{ToSpan, civil::Date};
use topcoat::{
    Result,
    view::{component, view},
};

use super::{
    META_LABEL,
    archive::scoring,
    filters::{LOG_PATH, MOVEMENT_DETAILS, MOVEMENTS, lookup},
    muscle_taxonomy,
};
use crate::util::urlencode;

const RECENT_DAYS: i64 = 7;
const BASELINE_DAYS: i64 = BASELINE_WEEKS as i64 * RECENT_DAYS;
const MIN_BASELINE_TRAINING_DAYS: usize = 4;
const MIN_MUSCLE_BASELINE_DAYS: usize = 2;

/// One immutable snapshot set projected into the small input this derivation
/// needs. Tags still ride along for movement suggestions; muscle credit
/// comes entirely from the stored weights.
pub(super) struct TrainingSet<'a> {
    pub(super) date: &'a str,
    pub(super) exercise_name: &'a str,
    pub(super) set_type: &'a str,
    pub(super) effort_hundredths: Option<u64>,
    pub(super) failure: bool,
    pub(super) tags: Option<&'a [(String, String)]>,
    /// `(granular muscle id, ratio_hundredths)` in canonical order, from
    /// `Snapshot::exercise_weight_map`.
    pub(super) weights: Option<&'a [(&'static str, u32)]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrainingFocus {
    pub(super) through_date: Date,
    /// Canonical order; configured targets appear even without any history.
    pub(super) muscles: Vec<MuscleLoad>,
    /// The complete signed vector, after regularity and recovery gates.
    pub(super) muscle_deltas: MuscleDeltas,
    pub(super) recommendation: Option<FocusRecommendation>,
    /// Usual-based recommendations wait for sufficient history; explicit
    /// targets do not depend on this flag.
    pub(super) baseline_ready: bool,
    /// A regular muscle is behind pace, but every such candidate was touched
    /// today or yesterday and is intentionally not prescribed again yet.
    pub(super) recovery_limited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MuscleLoad {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    /// Centi-points (volume points × ratio_hundredths) keep weighted credit
    /// exact without floats.
    pub(super) recent_centi_points: u32,
    /// Total centi-points across all eight baseline weeks. Divide by eight
    /// to compare it with one recent week.
    pub(super) baseline_centi_points: u32,
    pub(super) target_centi_points: Option<u32>,
}

impl MuscleLoad {
    fn deficit_scaled(&self) -> u32 {
        self.delta_scaled().clamp(0, i64::from(u32::MAX)) as u32
    }

    fn delta_scaled(&self) -> i64 {
        delta_scaled(
            self.target_centi_points,
            self.baseline_centi_points,
            self.recent_centi_points,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FocusRecommendation {
    pub(super) muscle_id: &'static str,
    pub(super) muscle_label: &'static str,
    /// The target (or usual) deficit on the common eight-week scale, unrounded.
    pub(super) deficit_scaled: u32,
    pub(super) target_based: bool,
    pub(super) movements: Vec<MovementSuggestion>,
    pub(super) exercises: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MovementSuggestion {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
}

#[derive(Clone, Copy, Debug, Default)]
struct PeriodVolume {
    recent: u32,
    baseline: u32,
}

impl PeriodVolume {
    fn add(&mut self, period: Period, centi_points: u32) {
        match period {
            Period::Recent => self.recent = self.recent.saturating_add(centi_points),
            Period::Baseline => self.baseline = self.baseline.saturating_add(centi_points),
        }
    }
}

#[derive(Clone, Copy)]
enum Period {
    Recent,
    Baseline,
}

pub(super) fn derive<'a>(
    sets: impl IntoIterator<Item = TrainingSet<'a>>,
    today: Date,
    targets: &HashMap<&'static str, u32>,
) -> TrainingFocus {
    let recent_start = today
        .checked_add((-(RECENT_DAYS - 1)).days())
        .expect("seven-day focus window is representable");
    let baseline_end = recent_start
        .checked_add((-1).days())
        .expect("focus baseline end is representable");
    let baseline_start = baseline_end
        .checked_add((-(BASELINE_DAYS - 1)).days())
        .expect("eight-week focus baseline is representable");

    let mut by_muscle: HashMap<&'static str, PeriodVolume> = HashMap::new();
    let mut exercise_weights = HashMap::new();
    let mut exercise_volume: HashMap<&str, PeriodVolume> = HashMap::new();
    let mut movements_by_exercise: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut baseline_training_dates = BTreeSet::new();
    let mut baseline_dates_by_muscle: HashMap<&'static str, BTreeSet<Date>> = HashMap::new();
    let mut last_recent_date_by_muscle: HashMap<&'static str, Date> = HashMap::new();

    for set in sets {
        let Ok(date) = set.date.parse::<Date>() else {
            continue;
        };
        let period = if (recent_start..=today).contains(&date) {
            Period::Recent
        } else if (baseline_start..=baseline_end).contains(&date) {
            Period::Baseline
        } else {
            continue;
        };
        let points = scoring::set_volume_points(set.set_type, set.effort_hundredths, set.failure);
        if points == 0 {
            continue;
        }
        if matches!(period, Period::Baseline) {
            baseline_training_dates.insert(date);
        }

        let weights = set.weights.unwrap_or_default();
        if weights.is_empty() {
            continue;
        }
        exercise_weights.insert(set.exercise_name, weights);
        exercise_volume
            .entry(set.exercise_name)
            .or_default()
            .add(period, points);
        let movements: BTreeSet<&'static str> = set
            .tags
            .unwrap_or_default()
            .iter()
            .filter(|(kind, _)| kind == "movement")
            .filter_map(|(_, value)| canonical_movement(value))
            .collect();
        movements_by_exercise
            .entry(set.exercise_name.to_string())
            .or_default()
            .extend(&movements);

        for (muscle, ratio) in weights {
            let centi_points = scoring::muscle_credit_centi(
                set.set_type,
                set.effort_hundredths,
                set.failure,
                *ratio,
            );
            by_muscle
                .entry(*muscle)
                .or_default()
                .add(period, centi_points);
            if matches!(period, Period::Baseline) {
                baseline_dates_by_muscle
                    .entry(*muscle)
                    .or_default()
                    .insert(date);
            } else {
                last_recent_date_by_muscle
                    .entry(*muscle)
                    .and_modify(|last| *last = (*last).max(date))
                    .or_insert(date);
            }
        }
    }

    let muscles: Vec<MuscleLoad> = muscle_taxonomy::muscles()
        .filter_map(|(id, label)| {
            let volume = by_muscle.get(id).copied().unwrap_or_default();
            let target_centi_points = targets.get(id).copied();
            (volume.recent > 0 || volume.baseline > 0 || target_centi_points.is_some()).then_some(
                MuscleLoad {
                    id,
                    label,
                    recent_centi_points: volume.recent,
                    baseline_centi_points: volume.baseline,
                    target_centi_points,
                },
            )
        })
        .collect();
    let baseline_ready = baseline_training_dates.len() >= MIN_BASELINE_TRAINING_DAYS
        && baseline_dates_by_muscle
            .values()
            .any(|dates| dates.len() >= MIN_MUSCLE_BASELINE_DAYS);
    let rest_cutoff = today
        .checked_add((-1).days())
        .expect("focus recovery cutoff is representable");
    let has_target = |muscle: &MuscleLoad| {
        muscle.target_centi_points.is_some()
            || (baseline_ready
                && baseline_dates_by_muscle
                    .get(muscle.id)
                    .is_some_and(|dates| dates.len() >= MIN_MUSCLE_BASELINE_DAYS))
    };
    let has_deficit = muscles
        .iter()
        .any(|muscle| has_target(muscle) && muscle.delta_scaled() > 0);
    let muscle_deltas: MuscleDeltas = muscles
        .iter()
        .filter(|muscle| has_target(muscle))
        .map(|muscle| {
            let delta = muscle.delta_scaled();
            // Recovery removes the incentive to train a fresh deficit, but
            // must not erase the penalty for an already over-target muscle.
            let recovering = last_recent_date_by_muscle
                .get(muscle.id)
                .is_some_and(|last| *last >= rest_cutoff);
            (
                muscle.id.to_string(),
                if recovering { delta.min(0) } else { delta },
            )
        })
        .collect();

    let mut exercises: Vec<(&str, i64)> = exercise_weights
        .iter()
        .map(|(name, weights)| (*name, dot_product(&muscle_deltas, weights)))
        .filter(|(_, score)| *score > 0)
        .collect();
    exercises.sort_unstable_by(|(left_name, left), (right_name, right)| {
        let left_volume = exercise_volume[left_name];
        let right_volume = exercise_volume[right_name];
        right
            .cmp(left)
            .then_with(|| right_volume.baseline.cmp(&left_volume.baseline))
            .then_with(|| right_volume.recent.cmp(&left_volume.recent))
            .then_with(|| left_name.cmp(right_name))
    });

    // Describe the strongest gap addressed by the best exercise. Without a
    // familiar matching exercise, an explicit target still gets a gap readout.
    let best_weights = exercises.first().map(|(name, _)| exercise_weights[name]);
    let recommendation = muscles
        .iter()
        .enumerate()
        .filter_map(|(index, muscle)| {
            let delta = muscle_deltas.get(muscle.id).copied().unwrap_or(0);
            let ratio = best_weights.map_or(100, |weights| {
                weights
                    .iter()
                    .find(|(id, _)| *id == muscle.id)
                    .map_or(0, |(_, ratio)| *ratio)
            });
            (delta > 0 && ratio > 0).then_some((index, muscle, delta * i64::from(ratio)))
        })
        .max_by(|(left_index, _, left), (right_index, _, right)| {
            left.cmp(right).then_with(|| right_index.cmp(left_index))
        })
        .map(|(_, muscle, _)| {
            let exercises: Vec<String> = exercises
                .iter()
                .take(2)
                .map(|(name, _)| (*name).to_string())
                .collect();
            // Movement links describe the ranked exercises; they never
            // pre-filter candidates before the full-vector comparison.
            let mut movements = Vec::new();
            for exercise in &exercises {
                let mut ids: Vec<_> = movements_by_exercise[exercise].iter().copied().collect();
                ids.sort_unstable_by_key(|id| movement_order(id));
                for id in ids {
                    if movements
                        .iter()
                        .any(|movement: &MovementSuggestion| movement.id == id)
                    {
                        continue;
                    }
                    if let Some(label) = canonical_movement_label(id) {
                        movements.push(MovementSuggestion { id, label });
                    }
                }
            }
            FocusRecommendation {
                muscle_id: muscle.id,
                muscle_label: muscle.label,
                deficit_scaled: muscle.deficit_scaled(),
                target_based: muscle.target_centi_points.is_some(),
                movements,
                exercises,
            }
        });
    let recovery_limited = has_deficit && recommendation.is_none();

    TrainingFocus {
        through_date: today,
        muscles,
        muscle_deltas,
        recommendation,
        baseline_ready,
        recovery_limited,
    }
}

fn canonical_movement(value: &str) -> Option<&'static str> {
    MOVEMENTS
        .iter()
        .chain(MOVEMENT_DETAILS)
        .find_map(|(id, _)| (*id == value).then_some(*id))
}

fn canonical_movement_label(value: &str) -> Option<&'static str> {
    lookup(MOVEMENTS, value).or_else(|| lookup(MOVEMENT_DETAILS, value))
}

fn movement_order(value: &str) -> usize {
    MOVEMENTS
        .iter()
        .chain(MOVEMENT_DETAILS)
        .position(|(id, _)| *id == value)
        .unwrap_or(usize::MAX)
}

fn load_groups(focus: &TrainingFocus) -> Vec<LoadGroup> {
    let scale = focus
        .muscles
        .iter()
        .map(|muscle| {
            muscle
                .recent_centi_points
                .saturating_mul(BASELINE_WEEKS)
                .max(muscle.baseline_centi_points)
                .max(
                    muscle
                        .target_centi_points
                        .unwrap_or(0)
                        .saturating_mul(BASELINE_WEEKS),
                )
        })
        .max()
        .unwrap_or(1)
        .max(1);
    // Group headers with granular bars beneath, in taxonomy display order;
    // a group with neither load nor targets is omitted entirely.
    muscle_taxonomy::MUSCLE_GROUPS
        .iter()
        .filter_map(|(_, group_label, members)| {
            let rows: Vec<LoadRow> = members
                .iter()
                .filter_map(|(id, _)| focus.muscles.iter().find(|muscle| muscle.id == *id))
                .map(|muscle| {
                    let recent_scaled = muscle.recent_centi_points.saturating_mul(BASELINE_WEEKS);
                    let recent_percent = percent(recent_scaled, scale);
                    let usual_percent = percent(muscle.baseline_centi_points, scale);
                    let recent = format_ratio(muscle.recent_centi_points, 100);
                    let usual = format_ratio(muscle.baseline_centi_points, BASELINE_WEEKS * 100);
                    let target = muscle
                        .target_centi_points
                        .map(|value| format_ratio(value, 100));
                    let target_percent = percent(
                        muscle
                            .target_centi_points
                            .unwrap_or(0)
                            .saturating_mul(BASELINE_WEEKS),
                        scale,
                    );
                    let target_description = target
                        .as_ref()
                        .map(|value| format!("weekly target {value} points"))
                        .unwrap_or_else(|| "no weekly target set".into());
                    LoadRow {
                        label: muscle.label,
                        href: muscle_url(muscle.id),
                        recent: recent.clone(),
                        usual: usual.clone(),
                        target,
                        style: format!(
                            "--muscle-recent-width: {recent_percent}%; \
                             --muscle-usual-left: {usual_percent}%; \
                             --muscle-target-left: {target_percent}%"
                        ),
                        accessible: format!(
                            "Recent load {recent} volume points in the past seven days; \
                             usual weekly pace {usual} points; {target_description}"
                        ),
                        has_baseline: muscle.baseline_centi_points > 0,
                    }
                })
                .collect();
            (!rows.is_empty()).then_some(LoadGroup {
                label: group_label,
                rows,
            })
        })
        .collect()
}

#[component]
pub(super) async fn panel(focus: &TrainingFocus, heading_id: &str, can_edit: bool) -> Result {
    let groups = load_groups(focus);
    let has_targets = focus
        .muscles
        .iter()
        .any(|muscle| muscle.target_centi_points.is_some());
    let through = focus.through_date.strftime("%b %-d").to_string();

    view! {
        <section aria-labelledby=(heading_id)>
            <p class=(META_LABEL)>"training compass"</p>
            if let Some(recommendation) = &focus.recommendation {
                <h2
                    id=(heading_id)
                    class="mt-1 font-display text-xl font-semibold leading-tight"
                >
                    "Next: "
                    if let Some(exercise) = recommendation.exercises.first() {
                        <a
                            class="text-oxide underline decoration-oxide/35 underline-offset-[0.18em]"
                            href=(exercise_url(exercise))
                        >
                            (exercise.as_str())
                        </a>
                    } else {
                        <a
                            class="text-oxide underline decoration-oxide/35 underline-offset-[0.18em]"
                            href=(muscle_url(recommendation.muscle_id))
                        >
                            (recommendation.muscle_label)
                        </a>
                    }
                </h2>
                <p class="mt-2 text-[0.8rem] leading-[1.55] text-ink2">
                    (recommendation.muscle_label)
                    ": about "
                    (format_ratio(recommendation.deficit_scaled, BASELINE_WEEKS * 100))
                    (if recommendation.target_based {
                        " volume points below its weekly target."
                    } else {
                        " volume points below its usual weekly pace."
                    })
                </p>
                if !recommendation.movements.is_empty() {
                    <p class=(format!("{META_LABEL} mt-3"))>"bias the next lift"</p>
                    <div class="mt-1.5 flex flex-wrap gap-1.5">
                        for movement in &recommendation.movements {
                            <a
                                class="rounded-full border border-oxide/35 bg-oxide/5 px-2 py-1 \
                                     font-meta text-[0.64rem] leading-none text-oxide \
                                     hover:border-oxide focus-visible:outline-solid \
                                     focus-visible:outline-2 focus-visible:outline-oxide \
                                     focus-visible:outline-offset-2"
                                href=(movement_url(movement.id))
                            >
                                (movement.label)
                            </a>
                        }
                    </div>
                }
                if recommendation.exercises.len() > 1 {
                    <p class=(format!("{META_LABEL} mt-3"))>"also fits"</p>
                    <ul class="mt-1 space-y-1 font-meta text-[0.7rem] leading-[1.45]">
                        for exercise in recommendation.exercises.iter().skip(1) {
                            <li>
                                <a
                                    class="text-ink2 underline decoration-hairline \
                                         underline-offset-[0.2em] hover:text-oxide \
                                         hover:decoration-oxide"
                                    href=(exercise_url(exercise))
                                >
                                    (exercise.as_str())
                                </a>
                            </li>
                        }
                    </ul>
                }
            } else if focus.recovery_limited {
                <h2
                    id=(heading_id)
                    class="mt-1 font-display text-xl font-semibold leading-tight"
                >
                    "Recover first"
                </h2>
                <p class="mt-2 text-[0.8rem] leading-[1.55] text-ink2">
                    "The muscles behind pace were touched today or yesterday. Give them room \
                     before chasing the gap."
                </p>
            } else if focus.baseline_ready || has_targets {
                <h2
                    id=(heading_id)
                    class="mt-1 font-display text-xl font-semibold leading-tight"
                >
                    (if has_targets { "On track" } else { "On your pace" })
                </h2>
                <p class="mt-2 text-[0.8rem] leading-[1.55] text-ink2">
                    (if has_targets {
                        "Your targets are met, and untargeted muscles with enough history are on pace. Let readiness pick the next lift."
                    } else {
                        "No regularly trained muscle is behind its usual week. Let readiness pick the next lift."
                    })
                </p>
            } else {
                <h2
                    id=(heading_id)
                    class="mt-1 font-display text-xl font-semibold leading-tight"
                >
                    "Building a baseline"
                </h2>
                <p class="mt-2 text-[0.8rem] leading-[1.55] text-ink2">
                    "This waits for four prior training days before it calls a next focus."
                </p>
            }

            <a
                class="space-entry-link"
                href=(super::exercise_space::page_url(focus.recommendation.as_ref().and_then(|recommendation| recommendation.exercises.first().map(String::as_str))))
            >"Explore exercises"</a>

            <div class="mt-5 border-t border-hairline pt-4">
                <header>
                    <div>
                        <p class=(META_LABEL)>"muscle load"</p>
                        <p class="mt-0.5 font-meta text-[0.62rem] text-muted">
                            "7 days through "
                            (through.as_str())
                        </p>
                    </div>
                    <p class="mt-2 text-right font-meta text-[0.58rem] uppercase tracking-[0.08em] text-muted">
                        "now / usual / target"
                    </p>
                </header>
                for group in &groups {
                    <p class=(format!("{META_LABEL} mt-3"))>(group.label)</p>
                    <ul class="mt-1.5 space-y-2.5">
                    for row in &group.rows {
                        <li>
                            <div class="flex items-baseline justify-between gap-2">
                                <a
                                    class="min-w-0 font-meta text-[0.68rem] text-ink2 \
                                         underline decoration-hairline underline-offset-[0.18em] \
                                         hover:text-oxide hover:decoration-oxide"
                                    href=(row.href.as_str())
                                >
                                    (row.label)
                                </a>
                                <span class="flex-none font-meta text-[0.62rem]" aria-hidden="true">
                                    <span class="text-ink">(row.recent.as_str())</span>
                                    <span class="text-muted">
                                        " / "
                                        (row.usual.as_str())
                                    </span>
                                    <span class="text-brass">
                                        " / "
                                        (row.target.as_deref().unwrap_or("–"))
                                    </span>
                                </span>
                                <span class="sr-only">(row.accessible.as_str())</span>
                            </div>
                            <div
                                class="relative mt-2.5 h-1 overflow-visible rounded-full bg-hairline"
                                style=(row.style.as_str())
                                aria-hidden="true"
                            >
                                <span
                                    class="absolute inset-y-0 left-0 \
                                         w-[var(--muscle-recent-width)] rounded-full bg-oxide/75"
                                ></span>
                                if row.has_baseline {
                                    <span
                                        class="absolute -bottom-0.5 -top-0.5 \
                                             left-[var(--muscle-usual-left)] w-px bg-patina"
                                    ></span>
                                }
                                if row.target.is_some() {
                                    <span
                                        class="absolute -top-2 left-[var(--muscle-target-left)] \
                                             size-1.5 -translate-x-1/2 rotate-45 border border-brass"
                                    ></span>
                                }
                            </div>
                        </li>
                    }
                    </ul>
                }
                <p class="mt-4 font-meta text-[0.6rem] leading-[1.5] text-muted">
                    <span class="text-oxide">"bar"</span>
                    " = now · "
                    <span class="text-patina">"tick"</span>
                    " = usual week"
                    " · "
                    <span class="text-brass">"diamond"</span>
                    " = weekly target"
                </p>
                if can_edit {
                    <a href="/admin/fitness-targets" class="mt-3 inline-block min-h-8 font-meta text-xs text-oxide underline underline-offset-4">
                        "Edit targets"
                    </a>
                }
            </div>
        </section>
    }
}

struct LoadGroup {
    label: &'static str,
    rows: Vec<LoadRow>,
}

struct LoadRow {
    label: &'static str,
    href: String,
    recent: String,
    usual: String,
    target: Option<String>,
    style: String,
    accessible: String,
    has_baseline: bool,
}

fn percent(value: u32, scale: u32) -> u32 {
    value
        .saturating_mul(100)
        .saturating_add(scale / 2)
        .checked_div(scale)
        .unwrap_or(0)
        .min(100)
}

/// Format `numerator / denominator` to at most one decimal, rounding
/// half-away-from-zero like the rest of the site's reader-facing numbers.
pub(super) fn format_ratio(numerator: u32, denominator: u32) -> String {
    let tenths = numerator
        .saturating_mul(10)
        .saturating_add(denominator / 2)
        .checked_div(denominator)
        .unwrap_or(0);
    if tenths.is_multiple_of(10) {
        (tenths / 10).to_string()
    } else {
        format!("{}.{:01}", tenths / 10, tenths % 10)
    }
}

/// Granular muscles link through their coarse tag facet — tags deliberately
/// stay at the original 13-value scale (`muscle_taxonomy::coarse_tag_for`).
fn muscle_url(id: &str) -> String {
    match muscle_taxonomy::coarse_tag_for(id) {
        Some(coarse) => format!("{LOG_PATH}?muscle={}#volume", urlencode(coarse)),
        None => format!("{LOG_PATH}#volume"),
    }
}

fn movement_url(id: &str) -> String {
    format!("{LOG_PATH}?movement={}#volume", urlencode(id))
}

fn exercise_url(name: &str) -> String {
    format!("{LOG_PATH}?exercise={}#volume", urlencode(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct OwnedSet {
        date: &'static str,
        exercise: &'static str,
        set_type: &'static str,
        effort: Option<u64>,
        failure: bool,
        tags: Vec<(String, String)>,
        weights: Vec<(&'static str, u32)>,
    }

    impl OwnedSet {
        fn sample(&self) -> TrainingSet<'_> {
            TrainingSet {
                date: self.date,
                exercise_name: self.exercise,
                set_type: self.set_type,
                effort_hundredths: self.effort,
                failure: self.failure,
                tags: Some(&self.tags),
                weights: Some(&self.weights),
            }
        }
    }

    fn tag(kind: &str, value: &str) -> (String, String) {
        (kind.to_string(), value.to_string())
    }

    fn bench(date: &'static str, set_type: &'static str, effort: Option<u64>) -> OwnedSet {
        OwnedSet {
            date,
            exercise: "Bench Press",
            set_type,
            effort,
            failure: false,
            tags: vec![tag("movement", "horizontal-push")],
            weights: vec![("mid-chest", 100), ("triceps", 50)],
        }
    }

    fn failure_bench(date: &'static str) -> OwnedSet {
        OwnedSet {
            failure: true,
            ..bench(date, "NORMAL_SET", None)
        }
    }

    fn squat(date: &'static str) -> OwnedSet {
        OwnedSet {
            date,
            exercise: "Full Squat",
            set_type: "NORMAL_SET",
            effort: Some(1000),
            failure: false,
            tags: vec![tag("movement", "squat-type")],
            weights: vec![("quads", 100), ("glute-max", 100)],
        }
    }

    fn shoulder(date: &'static str, exercise: &'static str, movement: &'static str) -> OwnedSet {
        OwnedSet {
            date,
            exercise,
            set_type: "NORMAL_SET",
            effort: Some(1000),
            failure: false,
            tags: vec![tag("movement", movement)],
            weights: vec![("lateral-delts", 100)],
        }
    }

    fn isolated(date: &'static str, exercise: &'static str, muscle: &'static str) -> OwnedSet {
        OwnedSet {
            date,
            exercise,
            set_type: "NORMAL_SET",
            effort: Some(1000),
            failure: false,
            tags: Vec::new(),
            weights: vec![(muscle, 100)],
        }
    }

    fn derive_owned(sets: &[OwnedSet]) -> TrainingFocus {
        derive(
            sets.iter().map(OwnedSet::sample),
            "2026-07-29".parse().unwrap(),
            &HashMap::new(),
        )
    }

    fn with_targets(sets: &[OwnedSet], targets: &[(&'static str, u32)]) -> TrainingFocus {
        derive(
            sets.iter().map(OwnedSet::sample),
            "2026-07-29".parse().unwrap(),
            &targets.iter().copied().collect(),
        )
    }

    #[test]
    fn targets_override_usual_and_clearing_restores_the_original_recommendation() {
        let sets = [
            squat("2026-06-01"),
            squat("2026-06-15"),
            squat("2026-07-01"),
            squat("2026-07-15"),
            bench("2026-06-02", "NORMAL_SET", Some(800)),
            bench("2026-06-16", "NORMAL_SET", Some(800)),
            bench("2026-07-02", "NORMAL_SET", Some(800)),
            bench("2026-07-16", "NORMAL_SET", Some(800)),
        ];
        let before = derive_owned(&sets);
        assert_eq!(before.recommendation.as_ref().unwrap().muscle_id, "quads");
        let raised = with_targets(&sets, &[("mid-chest", 1500)]);
        let pick = raised.recommendation.unwrap();
        assert_eq!(pick.muscle_id, "mid-chest");
        assert_eq!(pick.deficit_scaled, 12_000);
        assert!(pick.target_based);
        let lowered = with_targets(&sets, &[("quads", 0), ("glute-max", 0)]);
        assert_eq!(lowered.recommendation.unwrap().muscle_id, "mid-chest");
        assert_eq!(with_targets(&sets, &[]), before);
    }

    #[test]
    fn compound_coverage_can_beat_the_single_largest_gap() {
        let sets = [
            bench("2026-06-01", "NORMAL_SET", Some(1000)),
            isolated("2026-06-01", "Chest Fly", "mid-chest"),
            isolated("2026-06-01", "Curl", "biceps"),
        ];
        let focus = with_targets(
            &sets,
            &[("mid-chest", 1000), ("triceps", 1000), ("biceps", 1300)],
        );
        assert_eq!(focus.muscle_deltas.len(), 3);
        let pick = focus.recommendation.unwrap();
        assert_eq!(pick.exercises, ["Bench Press", "Curl"]);
        assert_eq!(pick.muscle_id, "mid-chest");
        assert!(focus.muscle_deltas["biceps"] > focus.muscle_deltas["mid-chest"]);
    }

    #[test]
    fn above_target_secondary_load_favors_a_more_selective_exercise() {
        let sets = [
            bench("2026-06-01", "NORMAL_SET", Some(1000)),
            isolated("2026-06-01", "Chest Fly", "mid-chest"),
            isolated("2026-07-29", "Triceps Extension", "triceps"),
        ];
        let focus = with_targets(&sets, &[("mid-chest", 1000), ("triceps", 0)]);
        assert_eq!(
            focus.muscle_deltas["triceps"], -4000,
            "recovery must retain surplus penalties"
        );
        assert_eq!(
            focus.recommendation.unwrap().exercises,
            ["Chest Fly", "Bench Press"]
        );
        let recovering = with_targets(&sets, &[("mid-chest", 1000), ("triceps", 2000)]);
        assert_eq!(
            recovering.muscle_deltas["triceps"], 0,
            "a fresh deficit earns no bonus"
        );
    }

    #[test]
    fn a_met_target_suppresses_an_otherwise_eligible_usual_deficit() {
        let mut sets = Vec::new();
        for date in ["2026-06-01", "2026-06-15", "2026-07-01", "2026-07-15"] {
            sets.extend(std::iter::repeat_n(isolated(date, "Curl", "biceps"), 4));
        }
        sets.push(isolated("2026-07-26", "Curl", "biceps"));
        assert!(derive_owned(&sets).recommendation.is_some());
        assert!(
            with_targets(&sets, &[("biceps", 500)])
                .recommendation
                .is_none()
        );
    }

    #[test]
    fn targets_need_no_history_but_keep_recovery_and_canonical_ties() {
        let focus = with_targets(&[], &[("quads", 1000), ("biceps", 1000), ("abs", 0)]);
        assert!(!focus.baseline_ready);
        assert_eq!(focus.muscles.len(), 3);
        assert_eq!(muscle(&focus, "abs").target_centi_points, Some(0));
        let pick = focus.recommendation.unwrap();
        assert_eq!(pick.muscle_id, "biceps");
        assert!(pick.movements.is_empty());
        assert!(pick.exercises.is_empty());
        for date in ["2026-07-28", "2026-07-29"] {
            let sets = [isolated(date, "Curl", "biceps")];
            let blocked = with_targets(&sets, &[("biceps", 2000)]);
            assert!(blocked.recommendation.is_none());
            assert!(blocked.recovery_limited);
            let fallback = with_targets(&sets, &[("biceps", 2000), ("quads", 1000)]);
            assert_eq!(fallback.recommendation.unwrap().muscle_id, "quads");
            assert!(!fallback.recovery_limited);
        }
    }

    #[test]
    fn target_recommendations_keep_familiar_picks_even_ahead_of_usual() {
        let sets = [bench("2026-07-26", "NORMAL_SET", Some(1000))];
        let focus = with_targets(&sets, &[("mid-chest", 2000)]);
        let pick = focus.recommendation.unwrap();
        assert_eq!(pick.movements[0].id, "horizontal-push");
        assert_eq!(pick.exercises, ["Bench Press"]);
        assert_eq!(pick.deficit_scaled, 12_000);
        assert!(derive_owned(&sets).recommendation.is_none());
    }

    #[test]
    fn load_scale_includes_targets_and_retains_zero_and_unset_descriptions() {
        let focus = with_targets(
            &[bench("2026-07-26", "NORMAL_SET", Some(1000))],
            &[("mid-chest", 1000), ("abs", 0)],
        );
        let groups = load_groups(&focus);
        let rows: Vec<_> = groups.iter().flat_map(|group| &group.rows).collect();
        let chest = rows.iter().find(|row| row.label == "mid chest").unwrap();
        assert!(chest.style.contains("--muscle-recent-width: 50%"));
        assert!(chest.style.contains("--muscle-target-left: 100%"));
        assert!(chest.accessible.contains("weekly target 10 points"));
        let triceps = rows.iter().find(|row| row.label == "triceps").unwrap();
        assert!(triceps.target.is_none());
        assert!(triceps.accessible.contains("no weekly target set"));
        let abs = rows.iter().find(|row| row.label == "abs").unwrap();
        assert_eq!(abs.target.as_deref(), Some("0"));
        assert!(abs.style.contains("--muscle-target-left: 0%"));
        let zero = load_groups(&with_targets(&[], &[("abs", 0)]));
        assert!(zero[0].rows[0].style.contains("--muscle-recent-width: 0%"));
    }

    fn muscle<'a>(focus: &'a TrainingFocus, id: &str) -> &'a MuscleLoad {
        focus
            .muscles
            .iter()
            .find(|muscle| muscle.id == id)
            .expect("muscle load")
    }

    #[test]
    fn ratios_scale_credit_and_warmups_earn_zero() {
        let focus = derive_owned(&[
            bench("2026-07-29", "NORMAL_SET", Some(1000)),
            bench("2026-07-29", "WARMUP_SET", Some(1000)),
            failure_bench("2026-07-23"),
        ]);

        // 5 + 0 + 6 = 11 points; mid-chest rides at 100, triceps at 50.
        assert_eq!(muscle(&focus, "mid-chest").recent_centi_points, 1100);
        assert_eq!(muscle(&focus, "triceps").recent_centi_points, 550);
        assert_eq!(focus.through_date.to_string(), "2026-07-29");
    }

    #[test]
    fn date_windows_are_inclusive_and_do_not_leak_old_or_future_sets() {
        let focus = derive_owned(&[
            bench("2026-07-23", "NORMAL_SET", Some(800)),
            bench("2026-07-22", "NORMAL_SET", Some(900)),
            bench("2026-05-28", "NORMAL_SET", Some(1000)),
            failure_bench("2026-05-27"),
            failure_bench("2026-07-30"),
        ]);

        assert_eq!(muscle(&focus, "mid-chest").recent_centi_points, 300);
        assert_eq!(muscle(&focus, "mid-chest").baseline_centi_points, 900);
    }

    #[test]
    fn recommendation_uses_personal_gap_and_observed_options() {
        let sets = [
            squat("2026-06-01"),
            squat("2026-06-15"),
            squat("2026-07-01"),
            squat("2026-07-15"),
            bench("2026-06-02", "NORMAL_SET", Some(800)),
            bench("2026-06-16", "NORMAL_SET", Some(800)),
            bench("2026-07-02", "NORMAL_SET", Some(800)),
            bench("2026-07-16", "NORMAL_SET", Some(800)),
            // Chest is already ahead of its baseline pace; quads/glutes are not.
            failure_bench("2026-07-27"),
        ];
        let focus = derive_owned(&sets);
        let recommendation = focus.recommendation.expect("recommendation");

        assert!(focus.baseline_ready);
        assert_eq!(recommendation.muscle_id, "quads", "canonical tie order");
        assert_eq!(recommendation.movements[0].id, "squat-type");
        assert_eq!(recommendation.exercises, vec!["Full Squat"]);
        assert!(recommendation.deficit_scaled > 0);
    }

    #[test]
    fn enough_recent_work_removes_a_muscle_from_contention() {
        let sets = [
            squat("2026-06-01"),
            squat("2026-06-15"),
            squat("2026-07-01"),
            squat("2026-07-15"),
            squat("2026-07-29"),
        ];
        let focus = derive_owned(&sets);

        assert!(focus.baseline_ready);
        assert!(
            focus.recommendation.is_none(),
            "one hard recent set exceeds this sparse routine's weekly pace"
        );
    }

    #[test]
    fn a_lagging_muscle_touched_yesterday_is_left_to_recover() {
        let mut recent = squat("2026-07-28");
        recent.effort = None;
        let focus = derive_owned(&[
            squat("2026-06-01"),
            squat("2026-06-15"),
            squat("2026-07-01"),
            squat("2026-07-15"),
            recent,
        ]);

        assert!(focus.baseline_ready);
        assert!(focus.recommendation.is_none());
        assert!(focus.recovery_limited);
    }

    #[test]
    fn recommendation_falls_through_to_the_largest_untouched_gap() {
        let mut recent_squat = squat("2026-07-28");
        recent_squat.effort = None;
        let focus = derive_owned(&[
            squat("2026-06-01"),
            squat("2026-06-01"),
            squat("2026-06-15"),
            squat("2026-06-15"),
            squat("2026-07-01"),
            squat("2026-07-01"),
            squat("2026-07-15"),
            squat("2026-07-15"),
            bench("2026-06-01", "NORMAL_SET", Some(800)),
            bench("2026-06-15", "NORMAL_SET", Some(800)),
            bench("2026-07-01", "NORMAL_SET", Some(800)),
            bench("2026-07-15", "NORMAL_SET", Some(800)),
            recent_squat,
        ]);
        let recommendation = focus.recommendation.expect("rested runner-up");

        assert_eq!(recommendation.muscle_id, "mid-chest");
        assert!(!focus.recovery_limited);
    }

    #[test]
    fn movement_links_follow_the_best_exercises_without_filtering_them_first() {
        let sets = [
            shoulder("2026-06-01", "Press A", "vertical-push"),
            shoulder("2026-06-08", "Press A", "vertical-push"),
            shoulder("2026-06-15", "Press A", "vertical-push"),
            shoulder("2026-06-01", "Press B", "vertical-push"),
            shoulder("2026-06-08", "Press B", "vertical-push"),
            shoulder("2026-06-15", "Press B", "vertical-push"),
            shoulder("2026-06-01", "Face Pull", "rear-delt"),
            shoulder("2026-06-08", "Face Pull", "rear-delt"),
            shoulder("2026-06-15", "Face Pull", "rear-delt"),
            shoulder("2026-06-22", "Face Pull", "rear-delt"),
            shoulder("2026-07-01", "Face Pull", "rear-delt"),
            // Equal muscle fits use exercise history to break ties, without
            // excluding a candidate because its movement used to rank third.
            shoulder("2026-06-01", "Lateral Raise", "shoulder-abduction"),
            shoulder("2026-06-08", "Lateral Raise", "shoulder-abduction"),
            shoulder("2026-06-15", "Lateral Raise", "shoulder-abduction"),
            shoulder("2026-06-22", "Lateral Raise", "shoulder-abduction"),
        ];
        let focus = derive_owned(&sets);
        let recommendation = focus.recommendation.expect("recommendation");

        assert_eq!(
            recommendation
                .movements
                .iter()
                .map(|movement| movement.id)
                .collect::<Vec<_>>(),
            vec!["rear-delt", "shoulder-abduction"]
        );
        assert_eq!(recommendation.exercises, vec!["Face Pull", "Lateral Raise"]);
    }

    #[test]
    fn sparse_or_untagged_history_never_prescribes() {
        let sparse = derive_owned(&[
            squat("2026-07-01"),
            squat("2026-07-15"),
            squat("2026-07-29"),
        ]);
        assert!(!sparse.baseline_ready);
        assert!(sparse.recommendation.is_none());

        let unweighted = [OwnedSet {
            date: "2026-07-29",
            exercise: "Mystery lift",
            set_type: "NORMAL_SET",
            effort: None,
            failure: true,
            tags: Vec::new(),
            weights: Vec::new(),
        }];
        let focus = derive_owned(&unweighted);
        assert!(focus.muscles.is_empty());
        assert!(focus.recommendation.is_none());
    }

    #[test]
    fn unrelated_one_off_days_do_not_establish_a_muscle_baseline() {
        let focus = derive_owned(&[
            isolated("2026-06-01", "Quad one-off", "quads"),
            isolated("2026-06-08", "Chest one-off", "mid-chest"),
            isolated("2026-06-15", "Back one-off", "lats"),
            isolated("2026-06-22", "Core one-off", "abs"),
        ]);

        assert!(!focus.baseline_ready);
        assert!(focus.recommendation.is_none());
    }
}
