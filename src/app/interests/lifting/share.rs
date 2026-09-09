//! Copy/paste sharing for one workout, rendered by the same canonical
//! application formatter as Podrick.
//!
//! The text is rendered server-side into a `<details>` disclosure whose
//! readonly `<textarea>` is always selectable — that is the
//! no-JavaScript path. The image preview and download use the same PNG as
//! Open Graph; `share.js` progressively enables text and image copying.

use benjisponge::workout_text::{self, Set as TextSet, Workout as TextWorkout};
use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{header, request::headers},
    view::{component, view},
};

use super::data as fitness;

pub(super) const SHARE_JS: Asset = asset!("./share.js");

// Tailwind vocab for the share block. Utilities stay whole per line for
// the build-time class scanner.
const SHARE_SUMMARY: &str = "w-fit py-[0.2rem] list-none [&::-webkit-details-marker]:hidden \
     text-oxide font-meta text-[0.72rem] cursor-pointer select-none \
     underline decoration-oxide/45 underline-offset-[0.24em] \
     group-open:decoration-current \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";
const SHARE_TEXT: &str = "block w-full max-h-[16rem] p-3 overflow-auto resize-y \
     whitespace-pre bg-page border border-hairline rounded-none \
     font-meta text-[0.7rem] leading-[1.6] text-ink2 outline-none \
     focus-visible:outline-solid focus-visible:outline-2 focus-visible:outline-oxide \
     focus-visible:outline-offset-2";
const SHARE_BUTTON: &str = "px-3 py-[0.45rem] font-meta text-[0.7rem] text-card bg-oxide \
     border border-oxide rounded-[0.2rem] cursor-pointer hover:text-white hover:bg-oxide-hot \
     hover:border-oxide-hot focus-visible:text-white focus-visible:bg-oxide-hot \
     focus-visible:border-oxide-hot disabled:opacity-60 disabled:cursor-wait";
const SHARE_IMAGE_LINK: &str = "font-meta text-[0.7rem] text-oxide underline \
     decoration-oxide/45 underline-offset-4 hover:decoration-current";
const SHARE_HINT: &str = "mt-2 font-meta text-[0.67rem] leading-[1.5] text-muted";

/// The absolute origin the visitor is browsing, mirroring the planes
/// receipt's QR: Host header plus forwarded scheme, or `None` when the
/// request names no host (the caller then shares the bare path).
pub(super) fn request_origin(cx: &Cx) -> Option<String> {
    let hdrs = headers(cx);
    let host = hdrs.get(header::HOST).and_then(|h| h.to_str().ok())?;
    let scheme = hdrs
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(
            if host.starts_with("localhost") || host.starts_with("127.") {
                "http"
            } else {
                "https"
            },
        );
    Some(format!("{scheme}://{host}"))
}

/// Adapt the archive API model into the canonical workout text model and add
/// the permanent URL for the current request origin.
pub(super) fn share_text(workout: &fitness::Workout, origin: Option<&str>) -> String {
    let permalink = format!("/fitness/lift/{}", workout.path);
    let permalink = origin.map_or(permalink.clone(), |origin| format!("{origin}{permalink}"));
    let workout = TextWorkout {
        title: workout.title.clone(),
        started_at_local: workout.started_at_local.clone(),
        ended_at_local: workout.ended_at_local.clone(),
        duration_seconds: i64::try_from(workout.duration_seconds)
            .expect("validated workout duration fits i64"),
        description: workout.description.clone(),
        notes: workout.notes.clone(),
        sets: workout
            .sets
            .iter()
            .map(|set| TextSet {
                exercise_name: set.exercise_name.clone(),
                set_type: set.set_type.clone(),
                superset_id: set
                    .superset_id
                    .map(|id| i64::try_from(id).expect("validated id")),
                weight_milli: set.weight_milli,
                weight_unit: set.weight_unit.clone(),
                reps: set
                    .reps
                    .map(|reps| i64::try_from(reps).expect("validated reps")),
                effort_hundredths: set
                    .effort_hundredths
                    .map(|effort| i64::try_from(effort).expect("validated effort")),
                failure: set.failure,
                distance_milli: set
                    .distance_milli
                    .map(|distance| i64::try_from(distance).expect("validated distance")),
                set_time_seconds: set
                    .set_time_seconds
                    .map(|seconds| i64::try_from(seconds).expect("validated set time")),
            })
            .collect(),
    };
    workout_text::format(&workout, &permalink, None)
}

/// The disclosure a workout page renders. `text` is prebuilt by the page
/// (it needs the request to know the public origin).
///
/// A readonly `<textarea>` rather than a `<pre>`: the em-dash response
/// layer (`src/emdash.rs`) leaves textarea content alone, so the sheet
/// stays copyable plain text even when a title or note contains an em
/// dash — and it is selectable without JavaScript.
#[component]
pub(super) async fn share_block(text: &str, image_url: &str, image_alt: &str) -> Result {
    let rows = text.lines().count().clamp(3, 14).to_string();
    view! {
        <details class="group" data-share="" data-share-image=(image_url)>
            <summary class=(SHARE_SUMMARY)>"share this workout"</summary>
            <div class="mt-2 max-w-[34rem]">
                <textarea
                    class=(SHARE_TEXT)
                    readonly=""
                    rows=(rows.as_str())
                    wrap="off"
                    spellcheck="false"
                    aria-label="Workout share text"
                >(text)</textarea>
                <div class="mt-2 flex flex-wrap items-center gap-2">
                    <button class=(SHARE_BUTTON) type="button" data-share-copy="" hidden="">
                        "copy text"
                    </button>
                    if !image_url.is_empty() {
                        <button class=(SHARE_BUTTON) type="button" data-share-copy-image="" hidden="">
                            "copy image"
                        </button>
                        <a class=(SHARE_IMAGE_LINK) href=(image_url) download="workout.png">
                            "save image"
                        </a>
                    }
                </div>
                <p class=(SHARE_HINT) data-share-hint="">
                    "Select the text above to copy it — it already ends with this page's link."
                </p>
                <p class=(SHARE_HINT) data-share-status="" role="status" aria-live="polite"></p>
                if !image_url.is_empty() {
                    <img
                        src=(image_url)
                        alt=(image_alt)
                        width="1200"
                        height=(super::social_card::HEIGHT.to_string())
                        loading="lazy"
                        decoding="async"
                        class="mt-3 block h-auto w-full border border-hairline"
                    >
                }
            </div>
        </details>
    }
}

/// A share row for the workout-page header list, so the disclosure sits
/// with the date/time/duration facts.
#[component]
pub(super) async fn share_row(text: &str, image_url: &str, image_alt: &str) -> Result {
    view! {
        <div class="rail-row">
            <dt class="rail-stamp rail-stamp-label">"share"</dt>
            <dd class="min-w-0">share_block(text: text, image_url: image_url, image_alt: image_alt)</dd>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(
        ordinal: u32,
        exercise: &str,
        weight: Option<i64>,
        reps: Option<u64>,
        effort: Option<u64>,
        set_type: &str,
        failure: bool,
    ) -> fitness::Set {
        fitness::Set {
            id: format!("s{ordinal}"),
            ordinal,
            exercise_name: exercise.to_string(),
            raw_exercise_name: exercise.to_string(),
            exercise_note: None,
            superset_id: None,
            weight_milli: weight,
            weight_unit: "lbs".into(),
            reps,
            effort_hundredths: effort,
            failure,
            distance_milli: None,
            set_time_seconds: None,
            set_type: set_type.into(),
            records: Vec::new(),
        }
    }

    fn workout() -> fitness::Workout {
        fitness::Workout {
            id: "fitness:2026-07-21T14:39:04".into(),
            path: "2026-07-21T10-39-04-04-00".into(),
            title: "I missed 9am gym".into(),
            raw_title: "I missed 9am gym".into(),
            started_at_local: "2026-07-21 10:39:04".into(),
            ended_at_local: "2026-07-21 11:14:14".into(),
            eastern_offset_minutes: -240,
            end_eastern_offset_minutes: -240,
            duration_seconds: 2110,
            duration_suspicious: false,
            notes: None,
            description: None,
            sets: vec![
                set(
                    1,
                    "Incline Bench Press",
                    Some(45_000),
                    Some(10),
                    None,
                    "WARMUP_SET",
                    false,
                ),
                set(
                    2,
                    "Incline Bench Press",
                    Some(145_000),
                    Some(3),
                    Some(800),
                    "NORMAL_SET",
                    false,
                ),
                set(
                    3,
                    "Cable Crossover",
                    Some(25_000),
                    Some(9),
                    None,
                    "NORMAL_SET",
                    true,
                ),
            ],
        }
    }

    #[test]
    fn share_text_lists_sets_and_ends_with_the_permalink() {
        let text = share_text(&workout(), Some("https://ben.soy"));
        let expected = "\
*I missed 9am gym*
Jul 21, 2026 · 10:39 AM–11:14 AM · 35m 10s · 2 working sets

I. Incline Bench Press
W. 45 lbs × 10
1. 145 lbs × 3 @ RPE 8

II. Cable Crossover
1. 25 lbs × 9 · failure

https://ben.soy/fitness/lift/2026-07-21T10-39-04-04-00";
        assert_eq!(text, expected);
    }

    #[test]
    fn share_text_without_an_origin_keeps_the_bare_path() {
        let text = share_text(&workout(), None);
        assert!(text.ends_with("\n/fitness/lift/2026-07-21T10-39-04-04-00"));
    }

    #[tokio::test]
    async fn share_image_preview_copy_and_download_use_the_same_url() {
        let cx = Cx::default();
        let __cx = &cx;
        let image_url = super::super::social_card::image_path("2026-07-21T10-39-04-04-00", 149);
        let html = view! {
            share_block(text: "Workout text", image_url: image_url.as_str(), image_alt: "Workout card")
        }.unwrap().render(__cx);
        let escaped_url = image_url.replace('&', "&amp;");
        assert!(html.contains(&format!("data-share-image=\"{escaped_url}\"")));
        assert!(html.contains(&format!("src=\"{escaped_url}\"")));
        assert!(html.contains(&format!("href=\"{escaped_url}\"")));
        assert!(html.contains("download=\"workout.png\""));
        assert!(html.contains("data-share-copy-image") && html.contains("copy image"));
        assert!(html.contains("Workout text</textarea>"));
    }

    #[test]
    fn share_text_keeps_negative_assistance() {
        let mut workout = workout();
        workout.sets[0].weight_milli = Some(-45_000);
        let text = share_text(&workout, None);
        assert!(text.contains("W. -45 lbs × 10"));
    }

    #[test]
    fn workout_notes_ride_along_and_records_stay_on_the_permalink() {
        let mut workout = workout();
        workout.description = Some("Deload week".into());
        workout.sets[1].records = vec![
            fitness::Record {
                level: "gold".into(),
                kind: "1rm".into(),
            },
            fitness::Record {
                level: "gold".into(),
                kind: "max-weight".into(),
            },
        ];
        workout.sets[2].records = vec![fitness::Record {
            level: "silver".into(),
            kind: "reps".into(),
        }];
        let text = share_text(&workout, None);
        assert!(text.contains("\nDeload week\n"));
        assert!(text.contains("1. 145 lbs × 3 @ RPE 8"));
        assert!(!text.contains("PR") && !text.contains("gold") && !text.contains("silver"));
    }
}
