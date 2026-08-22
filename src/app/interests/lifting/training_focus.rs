//! Page-only lifting load and next-focus guidance for `/fitness`.
//!
//! This is intentionally an approximation, not a hypertrophy prescription.
//! It scales the archive's effort-weighted volume score by each exercise's
//! stored muscle ratios (`exercise_muscles`, in hundredths), then compares
//! the last seven Eastern dates with this archive's own pace over the eight
//! preceding weeks. Credit accumulates in exact integer centi-points
//! (points × ratio_hundredths); display rounds once, half away from zero.

use std::collections::{BTreeSet, HashMap};

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
const BASELINE_WEEKS: u32 = 8;
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
    pub(super) tags: Option<&'a [(String, String)]>,
    /// `(granular muscle id, ratio_hundredths)` in canonical order, from
    /// `Snapshot::exercise_weight_map`.
    pub(super) weights: Option<&'a [(&'static str, u32)]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TrainingFocus {
    pub(super) through_date: Date,
    /// Canonical `MUSCLES` order; muscles absent from both windows are omitted.
    pub(super) muscles: Vec<MuscleLoad>,
    pub(super) recommendation: Option<FocusRecommendation>,
    /// A recommendation waits for several distinct baseline training dates.
    /// Recent volume can still render while this is false.
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FocusRecommendation {
    pub(super) muscle_id: &'static str,
    pub(super) muscle_label: &'static str,
    /// `baseline_centi_points - recent_centi_points * 8`; kept on the common
    /// eight-week scale so ranking never rounds.
    pub(super) deficit_scaled: u32,
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

    fn deficit_scaled(self) -> u32 {
        self.baseline
            .saturating_sub(self.recent.saturating_mul(BASELINE_WEEKS))
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
    let mut movement_by_muscle: HashMap<(&'static str, &'static str), PeriodVolume> =
        HashMap::new();
    let mut exercise_by_muscle: HashMap<(&'static str, String), PeriodVolume> = HashMap::new();
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
        let points = scoring::set_volume_points(set.set_type, set.effort_hundredths);
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
            let centi_points =
                scoring::muscle_credit_centi(set.set_type, set.effort_hundredths, *ratio);
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
            for movement in &movements {
                movement_by_muscle
                    .entry((*muscle, *movement))
                    .or_default()
                    .add(period, centi_points);
            }
            exercise_by_muscle
                .entry((*muscle, set.exercise_name.to_string()))
                .or_default()
                .add(period, centi_points);
        }
    }

    let muscles: Vec<MuscleLoad> = muscle_taxonomy::muscles()
        .filter_map(|(id, label)| {
            let volume = by_muscle.get(id).copied().unwrap_or_default();
            (volume.recent > 0 || volume.baseline > 0).then_some(MuscleLoad {
                id,
                label,
                recent_centi_points: volume.recent,
                baseline_centi_points: volume.baseline,
            })
        })
        .collect();
    let baseline_ready = baseline_training_dates.len() >= MIN_BASELINE_TRAINING_DAYS
        && baseline_dates_by_muscle
            .values()
            .any(|dates| dates.len() >= MIN_MUSCLE_BASELINE_DAYS);
    let rest_cutoff = today
        .checked_add((-1).days())
        .expect("focus recovery cutoff is representable");
    let is_regular_deficit = |muscle: &MuscleLoad| {
        baseline_dates_by_muscle
            .get(muscle.id)
            .is_some_and(|dates| dates.len() >= MIN_MUSCLE_BASELINE_DAYS)
            && PeriodVolume {
                recent: muscle.recent_centi_points,
                baseline: muscle.baseline_centi_points,
            }
            .deficit_scaled()
                > 0
    };
    let has_regular_deficit = baseline_ready && muscles.iter().any(is_regular_deficit);

    let recommendation = baseline_ready
        .then(|| {
            muscles
                .iter()
                .enumerate()
                .filter(|(_, muscle)| is_regular_deficit(muscle))
                .filter(|(_, muscle)| {
                    last_recent_date_by_muscle
                        .get(muscle.id)
                        .is_none_or(|last| *last < rest_cutoff)
                })
                .filter_map(|(index, muscle)| {
                    let volume = PeriodVolume {
                        recent: muscle.recent_centi_points,
                        baseline: muscle.baseline_centi_points,
                    };
                    let deficit = volume.deficit_scaled();
                    (deficit > 0).then_some((index, muscle, deficit))
                })
                .max_by(|(left_index, _, left), (right_index, _, right)| {
                    left.cmp(right)
                        // Reverse the index comparison so canonical order wins
                        // an exact deficit tie under `max_by`.
                        .then_with(|| right_index.cmp(left_index))
                })
        })
        .flatten()
        .map(|(_, muscle, deficit_scaled)| {
            let mut movements: Vec<(&'static str, PeriodVolume)> = movement_by_muscle
                .iter()
                .filter_map(|((candidate_muscle, movement), volume)| {
                    (*candidate_muscle == muscle.id
                        && volume.baseline > 0
                        && volume.deficit_scaled() > 0)
                        .then_some((*movement, *volume))
                })
                .collect();
            movements.sort_unstable_by(|(left_id, left), (right_id, right)| {
                right
                    .deficit_scaled()
                    .cmp(&left.deficit_scaled())
                    .then_with(|| right.baseline.cmp(&left.baseline))
                    .then_with(|| movement_order(left_id).cmp(&movement_order(right_id)))
            });
            movements.dedup_by_key(|(id, _)| *id);
            let movements = movements
                .into_iter()
                .take(2)
                .filter_map(|(id, _)| {
                    canonical_movement_label(id).map(|label| MovementSuggestion { id, label })
                })
                .collect::<Vec<_>>();
            let suggested_movement_ids: BTreeSet<&str> =
                movements.iter().map(|movement| movement.id).collect();

            let mut exercises: Vec<(String, PeriodVolume)> = exercise_by_muscle
                .iter()
                .filter_map(|((candidate_muscle, exercise), volume)| {
                    (*candidate_muscle == muscle.id
                        && volume.baseline > 0
                        && volume.deficit_scaled() > 0
                        && (suggested_movement_ids.is_empty()
                            || movements_by_exercise.get(exercise).is_some_and(
                                |candidate_movements| {
                                    candidate_movements
                                        .iter()
                                        .any(|movement| suggested_movement_ids.contains(movement))
                                },
                            )))
                    .then_some((exercise.clone(), *volume))
                })
                .collect();
            exercises.sort_unstable_by(|(left_name, left), (right_name, right)| {
                right
                    .deficit_scaled()
                    .cmp(&left.deficit_scaled())
                    .then_with(|| right.baseline.cmp(&left.baseline))
                    .then_with(|| left_name.cmp(right_name))
            });
            exercises.dedup_by(|(left, _), (right, _)| left == right);

            FocusRecommendation {
                muscle_id: muscle.id,
                muscle_label: muscle.label,
                deficit_scaled,
                movements,
                exercises: exercises
                    .into_iter()
                    .take(2)
                    .map(|(name, _)| name)
                    .collect(),
            }
        });
    let recovery_limited = has_regular_deficit && recommendation.is_none();

    TrainingFocus {
        through_date: today,
        muscles,
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

#[component]
pub(super) async fn panel(focus: &TrainingFocus, heading_id: &str) -> Result {
    let scale = focus
        .muscles
        .iter()
        .map(|muscle| {
            muscle
                .recent_centi_points
                .saturating_mul(BASELINE_WEEKS)
                .max(muscle.baseline_centi_points)
        })
        .max()
        .unwrap_or(1)
        .max(1);
    // Group headers with granular bars beneath, in taxonomy display order;
    // a group with no touched muscle is omitted entirely.
    let groups: Vec<LoadGroup> = muscle_taxonomy::MUSCLE_GROUPS
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
                    LoadRow {
                        label: muscle.label,
                        href: muscle_url(muscle.id),
                        recent: recent.clone(),
                        usual: usual.clone(),
                        style: format!(
                            "--muscle-recent-width: {recent_percent}%; \
                             --muscle-usual-left: {usual_percent}%"
                        ),
                        accessible: format!(
                            "Recent load {recent} volume points in the past seven days; \
                             usual weekly pace {usual} points"
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
        .collect();
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
                    <a
                        class="text-oxide underline decoration-oxide/35 underline-offset-[0.18em]"
                        href=(muscle_url(recommendation.muscle_id))
                    >
                        (recommendation.muscle_label)
                    </a>
                </h2>
                <p class="mt-2 text-[0.8rem] leading-[1.55] text-ink2">
                    "Largest rested gap: about "
                    (format_ratio(recommendation.deficit_scaled, BASELINE_WEEKS * 100))
                    " volume points below its usual weekly pace."
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
                if !recommendation.exercises.is_empty() {
                    <p class=(format!("{META_LABEL} mt-3"))>"familiar picks"</p>
                    <ul class="mt-1 space-y-1 font-meta text-[0.7rem] leading-[1.45]">
                        for exercise in &recommendation.exercises {
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
            } else if focus.baseline_ready {
                <h2
                    id=(heading_id)
                    class="mt-1 font-display text-xl font-semibold leading-tight"
                >
                    "On your pace"
                </h2>
                <p class="mt-2 text-[0.8rem] leading-[1.55] text-ink2">
                    "No regularly trained muscle is behind its usual week. Let readiness pick \
                     the next lift."
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

            <div class="mt-5 border-t border-hairline pt-4">
                <header class="flex items-end justify-between gap-3">
                    <div>
                        <p class=(META_LABEL)>"muscle load"</p>
                        <p class="mt-0.5 font-meta text-[0.62rem] text-muted">
                            "7 days through "
                            (through.as_str())
                        </p>
                    </div>
                    <p class="font-meta text-[0.58rem] uppercase tracking-[0.08em] text-muted">
                        "now / usual"
                    </p>
                </header>
                for group in &groups {
                    <p class=(format!("{META_LABEL} mt-3"))>(group.label)</p>
                    <ul class="mt-1.5 space-y-2.5">
                    for row in &group.rows {
                        <li>
                            <div class="flex items-baseline justify-between gap-2">
                                <a
                                    class="min-w-0 truncate font-meta text-[0.68rem] text-ink2 \
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
                                </span>
                                <span class="sr-only">(row.accessible.as_str())</span>
                            </div>
                            <div
                                class="relative mt-1 h-1 overflow-visible rounded-full bg-hairline"
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
                </p>
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
fn format_ratio(numerator: u32, denominator: u32) -> String {
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
            tags: vec![tag("movement", "horizontal-push")],
            weights: vec![("mid-chest", 100), ("triceps", 50)],
        }
    }

    fn squat(date: &'static str) -> OwnedSet {
        OwnedSet {
            date,
            exercise: "Full Squat",
            set_type: "NORMAL_SET",
            effort: Some(1000),
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
            tags: Vec::new(),
            weights: vec![(muscle, 100)],
        }
    }

    fn derive_owned(sets: &[OwnedSet]) -> TrainingFocus {
        derive(
            sets.iter().map(OwnedSet::sample),
            "2026-07-29".parse().unwrap(),
        )
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
            bench("2026-07-23", "FAILURE_SET", None),
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
            bench("2026-05-27", "FAILURE_SET", None),
            bench("2026-07-30", "FAILURE_SET", None),
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
            bench("2026-07-27", "FAILURE_SET", None),
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
    fn familiar_exercises_match_the_suggested_movement_types() {
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
            // This exercise has a larger individual gap than either press,
            // but its movement ranks third and therefore should not be shown
            // under the two suggested movement types.
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
            vec!["vertical-push", "rear-delt"]
        );
        assert_eq!(recommendation.exercises, vec!["Face Pull", "Press A"]);
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
            set_type: "FAILURE_SET",
            effort: None,
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
