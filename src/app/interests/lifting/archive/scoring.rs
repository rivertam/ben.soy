//! The set-log effort score.
//!
//! Deliberately an effort score, not load x reps. This is the single copy:
//! the calendar heatmap's daily totals and the per-set badges must stay in
//! exact lockstep (it used to be mirrored between the Worker's SQL and two
//! Rust view helpers).

use fitness_entry_core::SetType;

/// Parse the persisted/wire boundary once before applying the typed domain
/// score. Archive construction validates this value, so an unknown kind is a
/// broken invariant rather than a sixth fallback type.
pub fn set_volume_points(set_type: &str, effort_hundredths: Option<u64>, failure: bool) -> u32 {
    let set_type = set_type
        .parse::<SetType>()
        .expect("archive set_type was validated");
    fitness_entry_core::set_volume_points(set_type, effort_hundredths, failure)
}

pub fn effort_points(effort_hundredths: Option<u64>) -> u32 {
    set_volume_points("NORMAL_SET", effort_hundredths, false)
}

/// Weighted muscle credit for one set, in centi-points: the set's volume
/// points times the exercise↔muscle `ratio_hundredths` (1..=100). Exact
/// integers — accumulate centi-points and round only at display, half away
/// from zero like every other reader-facing number, so per-set rounding can
/// never drift across a few thousand sets.
pub fn muscle_credit_centi(
    set_type: &str,
    effort_hundredths: Option<u64>,
    failure: bool,
    ratio_hundredths: u32,
) -> u32 {
    set_volume_points(set_type, effort_hundredths, failure).saturating_mul(ratio_hundredths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_documented_scale() {
        assert_eq!(set_volume_points("NORMAL_SET", None, true), 6);
        assert_eq!(set_volume_points("WARMUP_SET", Some(1000), true), 0);
        assert_eq!(set_volume_points("NORMAL_SET", Some(1000), false), 5);
        assert_eq!(set_volume_points("NORMAL_SET", Some(900), false), 4);
        assert_eq!(set_volume_points("NORMAL_SET", Some(800), false), 3);
        assert_eq!(set_volume_points("NORMAL_SET", Some(750), false), 2);
        assert_eq!(set_volume_points("NORMAL_SET", None, false), 2);
        assert_eq!(set_volume_points("DROP_SET", Some(1000), false), 5);
    }

    #[test]
    fn muscle_credit_scales_points_by_ratio_exactly() {
        assert_eq!(
            muscle_credit_centi("NORMAL_SET", Some(1000), false, 100),
            500
        );
        assert_eq!(
            muscle_credit_centi("NORMAL_SET", Some(1000), false, 80),
            400
        );
        assert_eq!(muscle_credit_centi("NORMAL_SET", None, true, 50), 300);
        assert_eq!(muscle_credit_centi("WARMUP_SET", Some(1000), false, 100), 0);
        assert_eq!(muscle_credit_centi("NORMAL_SET", None, false, 33), 66);
    }
}
