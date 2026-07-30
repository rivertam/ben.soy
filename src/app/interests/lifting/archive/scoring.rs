//! The set-log effort score.
//!
//! Deliberately an effort score, not load x reps. This is the single copy:
//! the calendar heatmap's daily totals and the per-set badges must stay in
//! exact lockstep (it used to be mirrored between the Worker's SQL and two
//! Rust view helpers).

pub fn effort_points(effort_hundredths: Option<u64>) -> u32 {
    match effort_hundredths {
        Some(1000) => 5,
        Some(900) => 4,
        Some(800) => 3,
        _ => 2,
    }
}

pub fn set_volume_points(set_type: &str, effort_hundredths: Option<u64>) -> u32 {
    match set_type {
        "FAILURE_SET" => 6,
        "WARMUP_SET" => 0,
        _ => effort_points(effort_hundredths),
    }
}

/// Weighted muscle credit for one set, in centi-points: the set's volume
/// points times the exercise↔muscle `ratio_hundredths` (1..=100). Exact
/// integers — accumulate centi-points and round only at display, half away
/// from zero like every other reader-facing number, so per-set rounding can
/// never drift across a few thousand sets.
pub fn muscle_credit_centi(
    set_type: &str,
    effort_hundredths: Option<u64>,
    ratio_hundredths: u32,
) -> u32 {
    set_volume_points(set_type, effort_hundredths).saturating_mul(ratio_hundredths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_documented_scale() {
        assert_eq!(set_volume_points("FAILURE_SET", Some(500)), 6);
        assert_eq!(set_volume_points("WARMUP_SET", Some(1000)), 0);
        assert_eq!(set_volume_points("NORMAL_SET", Some(1000)), 5);
        assert_eq!(set_volume_points("NORMAL_SET", Some(900)), 4);
        assert_eq!(set_volume_points("NORMAL_SET", Some(800)), 3);
        assert_eq!(set_volume_points("NORMAL_SET", Some(750)), 2);
        assert_eq!(set_volume_points("NORMAL_SET", None), 2);
        assert_eq!(set_volume_points("DROP_SET", Some(1000)), 5);
    }

    #[test]
    fn muscle_credit_scales_points_by_ratio_exactly() {
        assert_eq!(muscle_credit_centi("NORMAL_SET", Some(1000), 100), 500);
        assert_eq!(muscle_credit_centi("NORMAL_SET", Some(1000), 80), 400);
        assert_eq!(muscle_credit_centi("FAILURE_SET", None, 50), 300);
        assert_eq!(muscle_credit_centi("WARMUP_SET", Some(1000), 100), 0);
        assert_eq!(muscle_credit_centi("NORMAL_SET", None, 33), 66);
    }
}
