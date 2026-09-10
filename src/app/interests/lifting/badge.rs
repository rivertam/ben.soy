//! A joined set stamp: the exercise-local number stays centered in its seal,
//! with inset service bars for effort and the exact label in its banner.

use std::fmt::Write;

use topcoat::{
    Result,
    view::{Unescaped, component, view},
};

use super::{archive::scoring::set_volume_points, data as fitness, format::format_scaled};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Badge {
    Warmup,
    Failure,
    Rated,
    Unrated,
}

pub(super) fn badge_for(set: &fitness::Set) -> Badge {
    if set.set_type == "WARMUP_SET" {
        Badge::Warmup
    } else if set.failure {
        Badge::Failure
    } else if set.effort_hundredths.is_some() {
        Badge::Rated
    } else {
        Badge::Unrated
    }
}

/// The HTML seal and raster share card use these same 36px drawing units.
/// `paper` is an application-authored CSS color, never workout text.
pub(super) fn seal_shapes(set: &fitness::Set, paper: &str) -> String {
    let badge = badge_for(set);
    let bars = set_volume_points(&set.set_type, set.effort_hundredths, set.failure);
    let dash = if matches!(badge, Badge::Warmup | Badge::Unrated) {
        "2 2"
    } else {
        "none"
    };
    let paint = if badge == Badge::Unrated {
        r#"fill="none" stroke="currentColor" stroke-width="0.7" opacity="0.65""#
    } else {
        r#"fill="currentColor""#
    };
    let mut svg = format!(
        r#"<circle cx="18" cy="18" r="17.75" fill="{paper}"/>
<circle class="lift-set-ring" cx="18" cy="18" r="17.2" fill="{paper}" stroke="currentColor" stroke-width="0.85" stroke-opacity="0.65" stroke-dasharray="{dash}"/>"#
    );
    for index in 0..bars {
        write!(
            svg,
            r#"<rect class="lift-set-bar" x="14.7" y="4.1" width="6.6" height="1.6" rx="0.2" transform="rotate({} 18 18)" {paint}/>"#,
            index * 360 / bars,
        )
        .expect("write to string");
    }
    svg
}

fn effort_label(effort: u64) -> String {
    // The archive accepts historical values beyond the entry form's RPE
    // range. Preserve those as recorded rather than inventing negative RIR.
    match 1_000_u64.checked_sub(effort) {
        Some(rir) => format!("{} RIR", format_scaled(rir, 100)),
        None => format!("RPE {}", format_scaled(effort, 100)),
    }
}

#[component]
pub(super) async fn set_badge(
    set: &fitness::Set,
    working_number: Option<usize>,
    effort_popover_id: &str,
) -> Result {
    let ordinal = working_number.map(|number| format!("{number:02}"));
    let number = ordinal.as_deref().unwrap_or("W");
    let badge = badge_for(set);
    let (state, banner) = match badge {
        Badge::Warmup => ("warmup", "warm-up".to_string()),
        Badge::Failure => ("failure", "failure".to_string()),
        Badge::Rated => ("rated", effort_label(set.effort_hundredths.unwrap())),
        Badge::Unrated => ("unrated", "unrated".to_string()),
    };
    let label = if badge == Badge::Warmup {
        "Warm-up set".to_string()
    } else {
        format!("Set {number}, {banner}")
    };
    let title = if badge == Badge::Failure {
        "Failure, with distinction."
    } else {
        label.as_str()
    };
    let anchor_name = format!("anchor-name: --inline-popover-{effort_popover_id};");
    let position_anchor = format!("position-anchor: --inline-popover-{effort_popover_id};");

    view! {
        if set.effort_hundredths.is_some() || badge == Badge::Failure {
            <button
                type="button"
                class="lift-set-stamp"
                data-set-effort=(state)
                popovertarget=(effort_popover_id)
                style=(anchor_name.as_str())
                title=(title)
                aria-label=(label.as_str())
            >
                stamp_face(set: set, number: number, banner: banner.as_str())
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
                if badge == Badge::Failure {
                    <span class="inline-popover-kicker">"failure"</span>
                    <span class="inline-popover-preview lift-set-commendation">"With distinction."</span>
                } else if let Some(effort) = set.effort_hundredths {
                    <span class="inline-popover-kicker">"RPE "(format_scaled(effort, 100))</span>
                    <span class="inline-popover-preview">
                        if let Some(rir) = 1_000_u64.checked_sub(effort) {
                            (format!("{} reps in reserve. ", format_scaled(rir, 100)))
                            "RIR describes how many more reps you could have completed."
                        } else {
                            "Rate of perceived exertion, as recorded for this set."
                        }
                    </span>
                }
            </span>
        } else {
            <span
                class="lift-set-stamp"
                data-set-effort=(state)
                role="img"
                title=(title)
                aria-label=(label.as_str())
            >
                stamp_face(set: set, number: number, banner: banner.as_str())
            </span>
        }
    }
}

#[component]
async fn stamp_face(set: &fitness::Set, number: &str, banner: &str) -> Result {
    let seal = seal_shapes(set, "var(--color-page)");
    view! {
        <span class="lift-set-number" aria-hidden="true">
            <svg class="lift-set-seal" viewBox="0 0 36 36" aria-hidden="true" focusable="false">
                (Unescaped::new_unchecked(seal))
            </svg>
            <span class="lift-set-numeral">(number)</span>
        </span>
        <span class="lift-set-effort" aria-hidden="true">(banner)</span>
    }
}

/// The social image's numbered seal, with the complete set available on tap.
/// The caller scopes IDs so a log and its day preview can coexist.
#[component]
pub(super) async fn compact_set_badge(
    row: &super::results::SetRow<'_>,
    popover_id: &str,
) -> Result {
    let set = row.set;
    let number = row
        .working_number
        .map_or_else(|| "W".to_string(), |n| format!("{n:02}"));
    let (state, effort) = match badge_for(set) {
        Badge::Warmup => ("warmup", "Warm-up set".to_string()),
        Badge::Failure => ("failure", format!("Set {number}, failure")),
        Badge::Rated => (
            "rated",
            format!(
                "Set {number}, {}",
                effort_label(set.effort_hundredths.unwrap())
            ),
        ),
        Badge::Unrated => ("unrated", format!("Set {number}, unrated")),
    };
    let label = format!(
        "{}, {effort}, {}{}",
        set.exercise_name,
        row.prescription,
        row.record
            .as_ref()
            .map_or(String::new(), |record| format!(", {record}"))
    );
    let seal = seal_shapes(set, "var(--color-card)");
    view! {
        <button
            type="button"
            class="lift-set-compact"
            data-set-effort=(state)
            popovertarget=(popover_id)
            style=(format!("anchor-name: --inline-popover-{popover_id};"))
            title=(label.as_str())
            aria-label=(label.as_str())
        >
            <svg class="lift-set-seal" viewBox="0 0 36 36" aria-hidden="true" focusable="false">
                (Unescaped::new_unchecked(seal))
            </svg>
            <span class="lift-set-numeral" aria-hidden="true">(number)</span>
            if row.record.is_some() {
                <span class="lift-set-compact-record" aria-hidden="true">"★"</span>
            }
        </button>
        <div
            id=(popover_id)
            class="inline-popover-panel"
            popover="auto"
            role="dialog"
            aria-label=(label.as_str())
            style=(format!("position-anchor: --inline-popover-{popover_id};"))
        >
            <button type="button" class="inline-popover-close" popovertarget=(popover_id)
                popovertargetaction="hide" aria-label="Close set details">"×"</button>
            <span class="inline-popover-kicker">(set.exercise_name.as_str())</span>
            <p class="text-ink font-semibold">(row.prescription.as_str())</p>
            <p>(effort)</p>
            if let Some(effort) = set.effort_hundredths {
                <p class="text-muted">"Recorded RPE "(format_scaled(effort, 100))</p>
            }
            if !row.details.is_empty() { <p>(row.details.as_str())</p> }
            if let Some(record) = &row.record { <p class="text-brass">(record.as_str())</p> }
            if let Some(note) = row.note { <p>(note)</p> }
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(set_type: &str, effort_hundredths: Option<u64>, failure: bool) -> fitness::Set {
        fitness::Set {
            id: "fitness:2026-07-21T21:03:00:0001".to_string(),
            ordinal: 6,
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
    fn rated_effort_preserves_exact_hundredths_and_historical_values() {
        assert_eq!(effort_label(1_000), "0 RIR");
        assert_eq!(effort_label(950), "0.5 RIR");
        assert_eq!(effort_label(925), "0.75 RIR");
        assert_eq!(effort_label(800), "2 RIR");
        assert_eq!(effort_label(600), "4 RIR");
        assert_eq!(effort_label(1_100), "RPE 11");
    }

    #[tokio::test]
    async fn working_numbers_remain_visible_for_every_effort_state() {
        let cx = topcoat::context::Cx::default();
        let __cx = &cx;
        for (effort, failure, banner, state, bars) in [
            (None, false, "unrated", "unrated", 2),
            (Some(1_000), false, "0 RIR", "rated", 5),
            (Some(900), false, "1 RIR", "rated", 4),
            (Some(800), false, "2 RIR", "rated", 3),
            (Some(700), false, "3 RIR", "rated", 2),
            (Some(950), false, "0.5 RIR", "rated", 2),
            (None, true, "failure", "failure", 6),
        ] {
            let source = set("NORMAL_SET", effort, failure);
            let html = view! {
                set_badge(set: &source, working_number: Some(3), effort_popover_id: "effort-test")
            }
            .unwrap()
            .render(__cx);
            assert!(html.contains(">03</span>"), "{html}");
            assert!(
                html.contains(&format!("aria-label=\"Set 03, {banner}\"")),
                "{html}"
            );
            assert!(
                html.contains(&format!("data-set-effort=\"{state}\"")),
                "{html}"
            );
            assert!(!html.contains("Set 06") && !html.contains('★'), "{html}");
            assert_eq!(html.matches("class=\"lift-set-bar\"").count(), bars);
            if effort.is_some() || failure {
                assert!(html.contains("popovertarget=\"effort-test\""));
                assert!(html.contains("id=\"effort-test\""));
            }
            if failure {
                assert!(html.contains("With distinction."));
                assert!(!html.contains("RPE "));
            }
        }
    }

    #[tokio::test]
    async fn warmups_keep_their_letter_and_banner_even_with_recorded_effort() {
        let cx = topcoat::context::Cx::default();
        let __cx = &cx;
        for effort in [None, Some(800)] {
            let source = set("WARMUP_SET", effort, false);
            let html = view! {
                set_badge(set: &source, working_number: None, effort_popover_id: "warmup-test")
            }
            .unwrap()
            .render(__cx);
            assert!(html.contains(">W</span>"), "{html}");
            assert!(html.contains(">warm-up</span>"), "{html}");
            assert!(html.contains("aria-label=\"Warm-up set\""));
            assert!(!html.contains("Set 01"));
            assert!(!html.contains("class=\"lift-set-bar\""));
            if effort.is_some() {
                assert!(html.contains("RPE 8"));
            }
        }
    }
}
