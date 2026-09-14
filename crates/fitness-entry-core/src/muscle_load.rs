//! Shared muscle-load geometry for the training compass and workout guide.

use std::collections::BTreeMap;

pub const BASELINE_WEEKS: u32 = 8;

/// Signed (target - current) centi-points on the eight-week baseline scale.
/// Missing muscles have no established target; zero means the target is met.
pub type MuscleDeltas = BTreeMap<String, i64>;

pub fn delta_scaled(target: Option<u32>, baseline: u32, recent: u32) -> i64 {
    target.map_or(i64::from(baseline), |target| {
        i64::from(target) * i64::from(BASELINE_WEEKS)
    }) - i64::from(recent) * i64::from(BASELINE_WEEKS)
}

/// Draft load uses the archive's centi-points, so scale it before subtraction.
/// Partial work reduces a need gradually and can take it below zero.
pub fn remaining_deltas(
    deltas: &MuscleDeltas,
    session_load: &BTreeMap<String, u32>,
) -> MuscleDeltas {
    deltas
        .iter()
        .map(|(muscle, delta)| {
            let load = i64::from(*session_load.get(muscle).unwrap_or(&0));
            (
                muscle.clone(),
                delta.saturating_sub(load * i64::from(BASELINE_WEEKS)),
            )
        })
        .collect()
}

/// Keep the stored exercise ratios unnormalized: covering several gaps adds
/// value, while loading muscles above target subtracts value. The common
/// hundredths factor does not affect ordering.
pub fn dot_product<S: AsRef<str>>(deltas: &MuscleDeltas, weights: &[(S, u32)]) -> i64 {
    weights.iter().fold(0_i64, |score, (muscle, ratio)| {
        score.saturating_add(
            deltas
                .get(muscle.as_ref())
                .copied()
                .unwrap_or(0)
                .saturating_mul(i64::from(*ratio)),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broad_gaps_favor_compounds_but_surpluses_favor_isolation() {
        let mut deltas = MuscleDeltas::from([("chest".into(), 800), ("triceps".into(), 800)]);
        let press = [("chest", 100), ("triceps", 50)];
        let fly = [("chest", 100)];
        assert!(dot_product(&deltas, &press) > dot_product(&deltas, &fly));
        deltas.insert("triceps".into(), -800);
        assert!(dot_product(&deltas, &press) < dot_product(&deltas, &fly));
    }

    #[test]
    fn session_credit_reduces_the_gap_in_the_same_units_without_rounding() {
        let deltas = MuscleDeltas::from([("triceps".into(), delta_scaled(None, 2001, 200))]);
        assert_eq!(deltas["triceps"], 401);
        let remaining = remaining_deltas(&deltas, &BTreeMap::from([("triceps".into(), 50)]));
        assert_eq!(remaining["triceps"], 1);
        let remaining = remaining_deltas(&deltas, &BTreeMap::from([("triceps".into(), 100)]));
        assert_eq!(remaining["triceps"], -399);
        assert_eq!(delta_scaled(Some(0), 2001, 200), -1600);
    }
}
