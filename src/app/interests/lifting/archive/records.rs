//! Historical PR-at-the-time record derivation.
//!
//! Records are derived from set history, never stored (decision 2026-07-22:
//! the denormalized `set_records` table is gone, and Lyfta's imported medals
//! are not the reference — these semantics stand on their own tests).
//!
//! A set earns a badge for a kind when, at the moment it was performed, its
//! metric ranked in the all-time top 3 for that exercise: gold beat every
//! earlier eligible set, silver all but one, bronze all but two. Badges are
//! frozen — later sets never demote them. Ties lose: matching an earlier
//! metric ranks strictly below it, so the first achiever keeps the medal.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Gold,
    Silver,
    Bronze,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Gold => "gold",
            Level::Silver => "silver",
            Level::Bronze => "bronze",
        }
    }

    fn from_rank(rank: usize) -> Option<Self> {
        match rank {
            1 => Some(Level::Gold),
            2 => Some(Level::Silver),
            3 => Some(Level::Bronze),
            _ => None,
        }
    }
}

/// Badge kinds, in the fixed order they render on a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    OneRm,
    MaxWeight,
    Volume,
    Reps,
}

pub const KIND_ORDER: [Kind; 4] = [Kind::OneRm, Kind::MaxWeight, Kind::Volume, Kind::Reps];

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::OneRm => "1rm",
            Kind::MaxWeight => "max-weight",
            Kind::Volume => "volume",
            Kind::Reps => "reps",
        }
    }

    fn index(self) -> usize {
        match self {
            Kind::OneRm => 0,
            Kind::MaxWeight => 1,
            Kind::Volume => 2,
            Kind::Reps => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Badge {
    pub level: Level,
    pub kind: Kind,
}

/// The current all-time winner for one exercise and record kind.
///
/// Unlike [`Badge`], which is frozen on the set that earned it, this points
/// only at the best eligible set after considering the complete history.
/// The source prescription rides along so entry views can show the useful PR
/// without looking the set up again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentBest {
    pub kind: Kind,
    pub set_id: String,
    pub weight_milli: Option<i64>,
    pub reps: Option<i64>,
}

/// The fields of a set that record derivation reads. Callers must supply
/// sets in chronological order: (started_at_utc, workout id, ordinal).
#[derive(Clone, Copy, Debug)]
pub struct SetSource<'a> {
    pub id: &'a str,
    pub exercise_name: &'a str,
    pub set_type: &'a str,
    pub weight_milli: Option<i64>,
    pub reps: Option<i64>,
}

/// Set types that neither earn records nor count as history to beat.
const EXCLUDED_SET_TYPES: [&str; 1] = ["WARMUP_SET"];

/// Per-kind comparable metric. Values only ever compete within one kind, so
/// scale factors are free; Epley 1RM (`w * (1 + reps/30)`) is compared as
/// `w * (30 + reps)` — same ordering, exact integer math.
fn metric(kind: Kind, weight_milli: Option<i64>, reps: Option<i64>) -> Option<i128> {
    match kind {
        Kind::MaxWeight => weight_milli.filter(|weight| *weight >= 0).map(i128::from),
        Kind::Reps => reps.map(i128::from),
        Kind::Volume => match (weight_milli, reps) {
            (Some(weight), Some(reps)) if weight >= 0 => {
                Some(i128::from(weight) * i128::from(reps))
            }
            _ => None,
        },
        Kind::OneRm => match (weight_milli, reps) {
            // A 1RM estimate needs at least one performed rep; at zero reps
            // Epley degenerates into max-weight. Negative weights represent
            // assistance, not resistance, so weight-derived metrics are not
            // meaningful or comparable to older positive-assistance history.
            // Reps remain useful.
            (Some(weight), Some(reps)) if weight >= 0 && reps >= 1 => {
                Some(i128::from(weight) * (30 + i128::from(reps)))
            }
            _ => None,
        },
    }
}

/// Top-3 metrics seen so far for one (exercise, kind), descending. Only the
/// podium matters for ranking; a set that fails to medal can never displace
/// a podium entry (it ranked below all of them).
#[derive(Clone, Debug, Default)]
struct Podium(Vec<i128>);

impl Podium {
    /// Rank the candidate against the podium (1 = beats everything) and
    /// admit it if it medals. Ties rank below the earlier achiever.
    fn rank_and_admit(&mut self, candidate: i128) -> Option<Level> {
        let beaten_by = self.0.iter().filter(|prior| **prior >= candidate).count();
        let level = Level::from_rank(beaten_by + 1)?;
        self.0.push(candidate);
        self.0.sort_unstable_by(|a, b| b.cmp(a));
        self.0.truncate(3);
        Some(level)
    }
}

/// Derive every badge in one chronological pass. Returns set id -> badges
/// (in `KIND_ORDER`); sets with no badges are absent.
pub fn derive<'a>(
    sets_in_chronological_order: impl IntoIterator<Item = SetSource<'a>>,
) -> HashMap<String, Vec<Badge>> {
    let mut podiums: HashMap<(String, usize), Podium> = HashMap::new();
    let mut badges: HashMap<String, Vec<Badge>> = HashMap::new();

    for set in sets_in_chronological_order {
        if EXCLUDED_SET_TYPES.contains(&set.set_type) {
            continue;
        }
        for kind in KIND_ORDER {
            let Some(candidate) = metric(kind, set.weight_milli, set.reps) else {
                continue;
            };
            let podium = podiums
                .entry((set.exercise_name.to_string(), kind.index()))
                .or_default();
            if let Some(level) = podium.rank_and_admit(candidate) {
                badges
                    .entry(set.id.to_string())
                    .or_default()
                    .push(Badge { level, kind });
            }
        }
    }

    badges
}

/// Derive the current all-time winner for every `(exercise, kind)` pair.
///
/// Callers supply sets in chronological order, just like [`derive`]. A
/// strictly larger metric replaces the winner; an equal metric does not, so
/// the earliest achiever keeps a tie. Each exercise's result follows
/// [`KIND_ORDER`] regardless of hash-map iteration order.
pub fn current_bests<'a>(
    sets_in_chronological_order: impl IntoIterator<Item = SetSource<'a>>,
) -> HashMap<String, Vec<CurrentBest>> {
    let mut bests: HashMap<String, [Option<(i128, CurrentBest)>; KIND_ORDER.len()]> =
        HashMap::new();

    for set in sets_in_chronological_order {
        if EXCLUDED_SET_TYPES.contains(&set.set_type) {
            continue;
        }
        for kind in KIND_ORDER {
            let Some(candidate) = metric(kind, set.weight_milli, set.reps) else {
                continue;
            };
            let by_kind = bests
                .entry(set.exercise_name.to_string())
                .or_insert_with(|| std::array::from_fn(|_| None));
            let winner = &mut by_kind[kind.index()];
            if winner
                .as_ref()
                .is_some_and(|(current, _)| *current >= candidate)
            {
                continue;
            }
            *winner = Some((
                candidate,
                CurrentBest {
                    kind,
                    set_id: set.id.to_string(),
                    weight_milli: set.weight_milli,
                    reps: set.reps,
                },
            ));
        }
    }

    bests
        .into_iter()
        .map(|(exercise, by_kind)| {
            let winners = by_kind
                .into_iter()
                .flatten()
                .map(|(_, best)| best)
                .collect();
            (exercise, winners)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set<'a>(
        id: &'a str,
        exercise: &'a str,
        set_type: &'a str,
        weight_milli: Option<i64>,
        reps: Option<i64>,
    ) -> SetSource<'a> {
        SetSource {
            id,
            exercise_name: exercise,
            set_type,
            weight_milli,
            reps,
        }
    }

    fn kinds(badges: &[Badge]) -> Vec<(&'static str, &'static str)> {
        badges
            .iter()
            .map(|badge| (badge.level.as_str(), badge.kind.as_str()))
            .collect()
    }

    #[test]
    fn first_set_of_an_exercise_is_gold_in_every_eligible_kind() {
        let derived = derive([set("a", "Squat", "NORMAL_SET", Some(225_000), Some(5))]);
        assert_eq!(
            kinds(&derived["a"]),
            [
                ("gold", "1rm"),
                ("gold", "max-weight"),
                ("gold", "volume"),
                ("gold", "reps"),
            ],
        );
    }

    #[test]
    fn current_winners_replace_frozen_historical_gold_badges() {
        let history = [
            set("first", "Squat", "NORMAL_SET", Some(200_000), Some(5)),
            set("later", "Squat", "NORMAL_SET", Some(225_000), Some(5)),
        ];
        let frozen = derive(history);
        assert!(
            frozen["first"]
                .iter()
                .any(|badge| badge.level == Level::Gold && badge.kind == Kind::MaxWeight)
        );
        assert!(
            frozen["later"]
                .iter()
                .any(|badge| badge.level == Level::Gold && badge.kind == Kind::MaxWeight)
        );

        let current = current_bests(history);
        let squat = &current["Squat"];
        assert_eq!(
            squat.iter().map(|best| best.kind).collect::<Vec<_>>(),
            KIND_ORDER
        );
        assert_eq!(
            squat
                .iter()
                .find(|best| best.kind == Kind::MaxWeight)
                .map(|best| best.set_id.as_str()),
            Some("later")
        );
        // Equal reps keep the earlier achiever even though the later set won
        // every weight-derived kind.
        assert_eq!(
            squat
                .iter()
                .find(|best| best.kind == Kind::Reps)
                .map(|best| best.set_id.as_str()),
            Some("first")
        );
    }

    #[test]
    fn current_winners_share_record_eligibility_and_earliest_tie_rules() {
        let current = current_bests([
            set("warmup", "Pull Up", "WARMUP_SET", Some(500_000), Some(100)),
            set(
                "assisted-first",
                "Pull Up",
                "NORMAL_SET",
                Some(-45_000),
                Some(12),
            ),
            set(
                "assisted-tie",
                "Pull Up",
                "NORMAL_SET",
                Some(-40_000),
                Some(12),
            ),
            set("weighted", "Pull Up", "NORMAL_SET", Some(25_000), Some(8)),
        ]);
        let pull_up = &current["Pull Up"];
        assert_eq!(
            pull_up.iter().map(|best| best.kind).collect::<Vec<_>>(),
            KIND_ORDER
        );
        for kind in [Kind::OneRm, Kind::MaxWeight, Kind::Volume] {
            assert_eq!(
                pull_up
                    .iter()
                    .find(|best| best.kind == kind)
                    .map(|best| best.set_id.as_str()),
                Some("weighted")
            );
        }
        let reps = pull_up
            .iter()
            .find(|best| best.kind == Kind::Reps)
            .expect("assisted reps are eligible");
        assert_eq!(reps.set_id, "assisted-first");
        assert_eq!(reps.weight_milli, Some(-45_000));
        assert_eq!(reps.reps, Some(12));
        assert!(pull_up.iter().all(|best| best.set_id != "warmup"));
    }

    #[test]
    fn badge_order_is_fixed_regardless_of_which_kinds_medal() {
        let derived = derive([
            set("a", "Squat", "NORMAL_SET", Some(200_000), Some(10)),
            // Heavier but lower volume/reps: medals only in 1rm + max-weight
            // (gold), plus silver volume; reps silver.
            set("b", "Squat", "NORMAL_SET", Some(300_000), Some(3)),
        ]);
        // a: 1rm 200k*40=8M, mw 200k, vol 2M, reps 10.
        // b: 1rm 300k*33=9.9M gold, mw gold, vol 900k silver, reps 3 silver.
        assert_eq!(
            kinds(&derived["b"]),
            [
                ("gold", "1rm"),
                ("gold", "max-weight"),
                ("silver", "volume"),
                ("silver", "reps"),
            ],
        );
    }

    #[test]
    fn ties_keep_the_first_achiever() {
        let derived = derive([
            set("a", "Bench", "NORMAL_SET", Some(185_000), Some(5)),
            set("b", "Bench", "NORMAL_SET", Some(185_000), Some(5)),
            set("c", "Bench", "NORMAL_SET", Some(185_000), Some(5)),
            set("d", "Bench", "NORMAL_SET", Some(185_000), Some(5)),
        ]);
        assert_eq!(derived["a"][0].level, Level::Gold);
        assert_eq!(derived["b"][0].level, Level::Silver);
        assert_eq!(derived["c"][0].level, Level::Bronze);
        assert!(
            !derived.contains_key("d"),
            "podium full of equal-or-better priors"
        );
    }

    #[test]
    fn badges_are_frozen_history() {
        let derived = derive([
            set("old", "Deadlift", "NORMAL_SET", Some(315_000), Some(1)),
            set("new", "Deadlift", "NORMAL_SET", Some(405_000), Some(1)),
        ]);
        // The old gold stays gold even though "new" out-lifts it.
        assert_eq!(derived["old"][0].level, Level::Gold);
        assert_eq!(derived["new"][0].level, Level::Gold);
    }

    #[test]
    fn warmups_neither_earn_nor_block() {
        let derived = derive([
            set("warm", "Squat", "WARMUP_SET", Some(500_000), Some(10)),
            set("work", "Squat", "NORMAL_SET", Some(225_000), Some(5)),
        ]);
        assert!(!derived.contains_key("warm"));
        assert_eq!(derived["work"][0].level, Level::Gold);
    }

    #[test]
    fn failure_sets_compete() {
        let derived = derive([
            set("a", "Curl", "FAILURE_SET", Some(50_000), Some(12)),
            set("b", "Curl", "NORMAL_SET", Some(50_000), Some(12)),
        ]);
        assert_eq!(derived["a"][0].level, Level::Gold);
        assert_eq!(derived["b"][0].level, Level::Silver);
    }

    #[test]
    fn zero_reps_is_no_one_rm_but_still_max_weight() {
        let derived = derive([set(
            "hold",
            "Farmer Carry",
            "NORMAL_SET",
            Some(140_000),
            Some(0),
        )]);
        assert_eq!(
            kinds(&derived["hold"]),
            [("gold", "max-weight"), ("gold", "volume"), ("gold", "reps")],
        );
    }

    #[test]
    fn bodyweight_sets_medal_in_reps_only() {
        let derived = derive([set("pullup", "Pull Up", "NORMAL_SET", None, Some(12))]);
        assert_eq!(kinds(&derived["pullup"]), [("gold", "reps")]);
    }

    #[test]
    fn negative_assistance_earns_reps_records_only() {
        let derived = derive([
            set(
                "more",
                "Pull Up (Assisted)",
                "NORMAL_SET",
                Some(-50_000),
                Some(5),
            ),
            set(
                "less",
                "Pull Up (Assisted)",
                "NORMAL_SET",
                Some(-25_000),
                Some(6),
            ),
        ]);
        assert_eq!(kinds(&derived["more"]), [("gold", "reps")]);
        assert_eq!(kinds(&derived["less"]), [("gold", "reps")]);
    }

    #[test]
    fn epley_favors_reps_at_equal_volume_sensibly() {
        // 100x10 (est 1rm 133) beats 120x3 (est 1rm 132).
        let derived = derive([
            set("a", "Press", "NORMAL_SET", Some(120_000), Some(3)),
            set("b", "Press", "NORMAL_SET", Some(100_000), Some(10)),
        ]);
        assert_eq!(
            derived["b"][0],
            Badge {
                level: Level::Gold,
                kind: Kind::OneRm
            }
        );
    }

    #[test]
    fn exercises_are_isolated() {
        let derived = derive([
            set("squat", "Squat", "NORMAL_SET", Some(315_000), Some(5)),
            set("lunge", "Lunge", "NORMAL_SET", Some(95_000), Some(8)),
        ]);
        assert_eq!(derived["lunge"][0].level, Level::Gold);
    }

    #[test]
    fn podium_shifts_down_as_records_improve() {
        let derived = derive([
            set("a", "Row", "NORMAL_SET", Some(100_000), Some(5)),
            set("b", "Row", "NORMAL_SET", Some(110_000), Some(5)),
            set("c", "Row", "NORMAL_SET", Some(120_000), Some(5)),
            set("d", "Row", "NORMAL_SET", Some(105_000), Some(5)),
        ]);
        // d beats only a (of the podium 120/110/100) in max-weight -> bronze.
        let d_max_weight = derived["d"]
            .iter()
            .find(|badge| badge.kind == Kind::MaxWeight)
            .unwrap();
        assert_eq!(d_max_weight.level, Level::Bronze);
    }
}
