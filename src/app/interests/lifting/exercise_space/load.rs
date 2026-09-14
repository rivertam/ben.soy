//! A fixed two-set preview on the compass's actual weekly load scale.
use fitness_entry_core::{SetType, set_volume_points};
use topcoat::{
    Result,
    view::{component, view},
};

use super::SpaceMuscle;

pub(super) fn preview_centi(ratio: u32) -> u32 {
    2 * set_volume_points(SetType::Normal, Some(900), false) * ratio
}

struct Bar {
    style: String,
    percentage: String,
    description: String,
    overflow: bool,
    unscaled: bool,
    usual: bool,
}

fn point_text(centi: f64) -> String {
    let tenths = (centi / 10.0).round() as u64;
    if tenths.is_multiple_of(10) {
        (tenths / 10).to_string()
    } else {
        format!("{}.{:01}", tenths / 10, tenths % 10)
    }
}

fn bar(muscle: &SpaceMuscle, delta_centi: u32, preview: bool) -> Bar {
    let current = f64::from(muscle.current_centi);
    let delta = f64::from(delta_centi);
    let reference = muscle.target_centi.map_or(muscle.usual_centi, f64::from);
    let scaled = reference > 0.0;
    let current_width = if scaled {
        (current / reference * 100.0).min(100.0)
    } else {
        0.0
    };
    let delta_width = if scaled {
        (delta / reference * 100.0).min(100.0 - current_width)
    } else {
        0.0
    };
    let usual = if scaled {
        (muscle.usual_centi / reference * 100.0).min(100.0)
    } else {
        0.0
    };
    let percentage = if scaled {
        let percent = ((if preview { delta } else { current }) / reference * 100.0).round() as u64;
        if preview && delta_centi > 0 {
            format!("+{percent}%")
        } else {
            format!("{percent}%")
        }
    } else if delta_centi > 0 {
        format!("+{} pt", point_text(delta))
    } else {
        "—".to_string()
    };
    let target = muscle.target_centi.map_or_else(
        || "no weekly target set".to_string(),
        |target| format!("weekly target {} points", point_text(f64::from(target))),
    );
    Bar {
        style: format!(
            "--space-current:{current_width:.4}%;--space-delta:{delta_width:.4}%;--space-usual:{usual:.4}%"
        ),
        percentage,
        description: format!(
            "{} points in the past seven days; +{} points from two sets at RPE 9; {} points after; {target}; usual weekly pace {} points.",
            point_text(current),
            point_text(delta),
            point_text(current + delta),
            point_text(muscle.usual_centi),
        ),
        overflow: scaled && current + delta > reference,
        unscaled: !scaled,
        usual: scaled && muscle.usual_centi > 0.0,
    }
}

fn row_order(muscles: &[SpaceMuscle], deltas: Option<&[u32]>) -> Vec<usize> {
    let mut order: Vec<_> = (0..muscles.len()).collect();
    if let Some(deltas) = deltas {
        let fraction = |index: usize| {
            let muscle = &muscles[index];
            let reference = muscle.target_centi.map_or(muscle.usual_centi, f64::from);
            if reference > 0.0 {
                f64::from(deltas[index]) / reference
            } else {
                0.0
            }
        };
        order.sort_by(|left, right| {
            fraction(*right)
                .total_cmp(&fraction(*left))
                .then_with(|| (deltas[*right] > 0).cmp(&(deltas[*left] > 0)))
                .then_with(|| left.cmp(right))
        });
    }
    order
}

#[component]
pub(super) async fn panel(muscles: &[SpaceMuscle], deltas: Option<&[u32]>) -> Result {
    view! {
        <div class="space-profile" data-space-profile="">
            <h3>"Muscle load"</h3>
            <p class="space-note" data-space-load-note="">(if deltas.is_some() { "2 sets at RPE 9 · added % of target" } else { "Select an exercise to preview 2 sets at RPE 9." })</p>
            <div class="space-load-legend" aria-label="Bar colors"><span><i data-kind="current"></i>"Trained"</span><span><i data-kind="delta"></i>"+2 sets"</span><span><i data-kind="empty"></i>"Remaining"</span></div>
            <ul class="space-load-rows">
                for index in row_order(muscles, deltas) {
                    row(muscle: &muscles[index], index: index, delta_centi: deltas.map_or(0, |deltas| deltas[index]), preview: deltas.is_some())
                }
            </ul>
            <p class="space-note space-load-key">"Bar end = weekly target, or usual pace if unset. Tick = usual. → = over target."</p>
        </div>
    }
}

#[component]
async fn row(muscle: &SpaceMuscle, index: usize, delta_centi: u32, preview: bool) -> Result {
    let bar = bar(muscle, delta_centi, preview);
    view! {
        <li data-space-load-row=(index) data-preview=(if delta_centi > 0 { "true" } else { "false" }) data-unscaled=(if bar.unscaled { "true" } else { "false" }) title=(bar.description.as_str())>
            <span>(muscle.label)</span>
            <span class="space-load-track" style=(bar.style.as_str()) aria-hidden="true">
                <i class="space-load-current"></i><i class="space-load-delta"></i>
                <i class="space-load-usual" hidden=(!bar.usual)></i>
                <i class="space-load-overflow" hidden=(!bar.overflow)>"›"</i>
            </span>
            <strong data-space-load-percent="" aria-hidden="true">(bar.percentage.as_str())</strong>
            <span class="sr-only" data-space-load-description="">(bar.description.as_str())</span>
        </li>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_rpe_nine_sets_add_weighted_load_against_the_fixed_target() {
        let muscle = SpaceMuscle {
            label: "chest",
            current_centi: 400,
            usual_centi: 1000.0,
            target_centi: Some(2000),
        };
        assert_eq!(preview_centi(100), 800);
        assert_eq!(preview_centi(50), 400);
        let preview = bar(&muscle, preview_centi(100), true);
        assert_eq!(preview.percentage, "+40%");
        assert!(
            preview
                .style
                .contains("--space-current:20.0000%;--space-delta:40.0000%")
        );
        assert!(!preview.overflow);
        assert!(preview.description.contains("+8 points"));
    }

    #[test]
    fn overshooting_keeps_the_target_scale_and_shows_only_the_added_percentage() {
        let mut muscle = SpaceMuscle {
            label: "chest",
            current_centi: 600,
            usual_centi: 500.0,
            target_centi: Some(1000),
        };
        let preview = bar(&muscle, 800, true);
        assert_eq!(preview.percentage, "+80%");
        assert!(preview.overflow);
        assert!(
            preview
                .style
                .contains("--space-current:60.0000%;--space-delta:40.0000%")
        );
        muscle.current_centi = 1500;
        let preview = bar(&muscle, 800, true);
        assert_eq!(preview.percentage, "+80%");
        assert!(
            preview
                .style
                .contains("--space-current:100.0000%;--space-delta:0.0000%")
        );
    }

    #[test]
    fn unset_uses_usual_but_zero_target_and_no_history_have_no_percent_scale() {
        let mut muscle = SpaceMuscle {
            label: "chest",
            current_centi: 400,
            usual_centi: 1000.0,
            target_centi: None,
        };
        assert_eq!(bar(&muscle, 400, true).percentage, "+40%");
        assert_eq!(bar(&muscle, 0, true).percentage, "0%");
        assert_eq!(bar(&muscle, 0, false).percentage, "40%");
        muscle.target_centi = Some(0);
        let preview = bar(&muscle, 400, true);
        assert!(preview.unscaled);
        assert_eq!(preview.percentage, "+4 pt");
        assert!(preview.description.contains("weekly target 0 points"));
        muscle.target_centi = None;
        muscle.usual_centi = 0.0;
        assert_eq!(bar(&muscle, 0, true).percentage, "—");
        assert!(!bar(&muscle, 400, true).style.contains("NaN"));
    }

    #[test]
    fn largest_relative_additions_sort_first_with_stable_ties_and_unaffected_last() {
        let muscles: Vec<_> = [Some(2000), Some(500), Some(1000), Some(0), Some(1000)]
            .into_iter()
            .map(|target_centi| SpaceMuscle {
                label: "muscle",
                current_centi: 0,
                usual_centi: 0.0,
                target_centi,
            })
            .collect();
        assert_eq!(
            row_order(&muscles, Some(&[800, 400, 0, 800, 800])),
            [1, 4, 0, 3, 2]
        );
        assert_eq!(row_order(&muscles, None), [0, 1, 2, 3, 4]);
    }
}
