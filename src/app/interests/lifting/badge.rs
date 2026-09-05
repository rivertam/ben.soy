//! The per-set badge: the small dial at the head of each set row. The badge
//! kind is decided entirely by the set's data — warm-ups get a dashed W,
//! failure sets a filled F, and working sets a ring of effort points.

use topcoat::{
    Result,
    view::{class, component, view},
};

use super::{data as fitness, format::format_scaled};

/// Which badge a set earns.
#[derive(Debug, PartialEq, Eq)]
enum Badge {
    Warmup,
    Failure,
    /// A working set: 2–5 stars around the dial, scored from recorded effort.
    /// Without a recorded effort the ordinal shows in the middle of the dial.
    Points {
        points: u32,
        show_ordinal: bool,
    },
}

fn badge_for(set: &fitness::Set) -> Badge {
    if set.set_type == "WARMUP_SET" {
        Badge::Warmup
    } else if set.failure {
        Badge::Failure
    } else {
        Badge::Points {
            points: effort_points(set.effort_hundredths),
            show_ordinal: set.effort_hundredths.is_none(),
        }
    }
}

/// Missing effort follows the intended low/default branch rather than
/// JavaScript's surprising `Number(null) == 0` coercion.
pub(super) use super::archive::scoring::effort_points;

fn star_angles(points: u32) -> Vec<String> {
    (0..points)
        .map(|index| format!("--point-angle: {}deg", index * 360 / points))
        .collect()
}

const DIAL: &str = "relative inline-flex size-8 items-center justify-center rounded-full border \
     font-meta text-[0.69rem] font-bold leading-none tracking-[0.04em]";
const DIAL_WARMUP: &str =
    "text-steel border-dashed border-[color-mix(in_srgb,currentColor_38%,transparent)]";
const DIAL_FAILURE: &str =
    "text-oxide bg-oxide/8 border-[color-mix(in_srgb,currentColor_38%,transparent)]";
const DIAL_POINTS: &str = "text-oxide border-oxide/18";
/// Each star is pinned to the dial's center, then rotated and swung out to
/// the ring by its `--point-angle`.
const STAR: &str = "absolute top-1/2 left-1/2 font-body text-[0.6rem] leading-none text-brass \
     [transform:translate(-50%,-50%)_rotate(var(--point-angle))_translateY(-0.48rem)]";

#[component]
pub(super) async fn set_badge(set: &fitness::Set, effort_popover_id: &str) -> Result {
    let ordinal = format!("{:02}", set.ordinal);
    let effort = set.effort_hundredths.map(|value| format_scaled(value, 100));
    let (dial, text, title, label, angles) = match badge_for(set) {
        Badge::Warmup => (
            DIAL_WARMUP,
            Some("W".to_string()),
            format!(
                "Set {ordinal} · warm-up{}",
                effort
                    .as_deref()
                    .map(|value| format!(" · RPE {value}"))
                    .unwrap_or_default()
            ),
            format!(
                "Set {ordinal}, warm-up{}",
                effort
                    .as_deref()
                    .map(|value| format!(", RPE {value}"))
                    .unwrap_or_default()
            ),
            Vec::new(),
        ),
        Badge::Failure => (
            DIAL_FAILURE,
            Some("F".to_string()),
            format!(
                "Set {ordinal} · failure{}",
                effort
                    .as_deref()
                    .map(|value| format!(" · RPE {value}"))
                    .unwrap_or_default()
            ),
            format!(
                "Set {ordinal}, failure{}",
                effort
                    .as_deref()
                    .map(|value| format!(", RPE {value}"))
                    .unwrap_or_default()
            ),
            Vec::new(),
        ),
        Badge::Points {
            points,
            show_ordinal,
        } => {
            let effort_label = effort
                .as_deref()
                .map(|value| format!("RPE {value}"))
                .unwrap_or_else(|| "RPE not recorded".to_string());
            (
                DIAL_POINTS,
                show_ordinal.then(|| ordinal.clone()),
                format!("Set {ordinal} · {effort_label} · {points} of 5 points"),
                format!("Set {ordinal}, {effort_label}, {points} of 5 points"),
                star_angles(points),
            )
        }
    };

    let effort_popover = effort;
    let anchor_name = format!("anchor-name: --inline-popover-{effort_popover_id};");
    let position_anchor = format!("position-anchor: --inline-popover-{effort_popover_id};");

    view! {
        if let Some(effort) = effort_popover {
            <button
                type="button"
                class=(class!(DIAL, dial, "appearance-none p-0 bg-transparent cursor-help hover:border-oxide focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide focus-visible:outline-offset-2"))
                popovertarget=(effort_popover_id)
                style=(anchor_name.as_str())
                title=(title.as_str())
                aria-label=(label.as_str())
            >
                for style in angles.iter() {
                    <span class=(STAR) style=(style.as_str()) aria-hidden="true">
                        "★"
                    </span>
                }
            </button>
            <span
                id=(effort_popover_id)
                class="inline-popover-panel"
                popover="auto"
                style=(position_anchor.as_str())
            >
                <button
                    type="button"
                    class="inline-popover-close"
                    popovertarget=(effort_popover_id)
                    popovertargetaction="hide"
                    aria-label="Close popover"
                >"×"</button>
                <span class="inline-popover-kicker">"RPE "(effort.as_str())</span>
                <span class="inline-popover-preview">
                    "Rate of perceived exertion. Strong values below 6 are treated as reps in reserve and converted to RPE during import."
                </span>
            </span>
        } else {
        <span
            class=(class!(DIAL, dial))
            role="img"
            title=(title.as_str())
            aria-label=(label.as_str())
        >
            if let Some(text) = &text {
                (text.as_str())
            }
            for style in angles.iter() {
                <span class=(STAR) style=(style.as_str()) aria-hidden="true">
                    "★"
                </span>
            }
        </span>
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(set_type: &str, effort_hundredths: Option<u64>, failure: bool) -> fitness::Set {
        fitness::Set {
            id: "fitness:2026-07-21T21:03:00:0001".to_string(),
            ordinal: 1,
            exercise_name: "Bench".to_string(),
            raw_exercise_name: "Bench".to_string(),
            exercise_note: None,
            superset_id: None,
            weight_milli: None,
            weight_unit: "lbs".to_string(),
            reps: None,
            effort_hundredths,
            failure,
            distance_milli: None,
            set_time_seconds: None,
            set_type: set_type.to_string(),
            records: Vec::new(),
        }
    }

    #[test]
    fn badge_kind_is_decided_by_the_set_data() {
        assert_eq!(
            badge_for(&set("WARMUP_SET", Some(1_000), false)),
            Badge::Warmup
        );
        assert_eq!(badge_for(&set("NORMAL_SET", None, true)), Badge::Failure);
        assert_eq!(
            badge_for(&set("NORMAL_SET", Some(1_000), false)),
            Badge::Points {
                points: 5,
                show_ordinal: false
            }
        );
        // A working set without recorded effort keeps its ordinal visible.
        assert_eq!(
            badge_for(&set("NORMAL_SET", None, false)),
            Badge::Points {
                points: 2,
                show_ordinal: true
            }
        );
        // Unknown set kinds fall through to the working-set dial.
        assert_eq!(
            badge_for(&set("DROP_SET", Some(900), false)),
            Badge::Points {
                points: 4,
                show_ordinal: false
            }
        );
    }

    #[test]
    fn missing_effort_is_not_coerced_to_zero() {
        assert_eq!(effort_points(None), 2);
        assert_eq!(effort_points(Some(1_000)), 5);
    }

    #[test]
    fn stars_are_spread_evenly_around_the_dial() {
        assert_eq!(
            star_angles(2),
            vec!["--point-angle: 0deg", "--point-angle: 180deg"]
        );
        assert_eq!(
            star_angles(4),
            vec![
                "--point-angle: 0deg",
                "--point-angle: 90deg",
                "--point-angle: 180deg",
                "--point-angle: 270deg"
            ]
        );
    }
}
