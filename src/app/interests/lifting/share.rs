//! Copy/paste sharing for one workout: a plain-text rendition of the
//! set log plus the workout's permanent URL, in the spirit of the
//! Strong/Lyfta share sheets.
//!
//! The text is rendered server-side into a `<details>` disclosure whose
//! readonly `<textarea>` is always selectable — that is the
//! no-JavaScript path. `share.js` progressively reveals a clipboard
//! button, exactly like `auto-filter.js` upgrades the log filter chrome.

use topcoat::{
    Result,
    asset::{Asset, asset},
    context::Cx,
    router::{header, headers},
    view::{component, view},
};

use super::{
    data as fitness,
    format::{format_integer, format_scaled, plural},
    results::WorkoutCard,
};

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
const SHARE_BUTTON: &str = "mt-2 px-3 py-[0.45rem] font-meta text-[0.7rem] text-card bg-oxide \
     border border-oxide rounded-[0.2rem] cursor-pointer hover:text-white hover:bg-oxide-hot \
     hover:border-oxide-hot focus-visible:text-white focus-visible:bg-oxide-hot \
     focus-visible:border-oxide-hot";
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

/// The plain-text share sheet: title, meta line, every set grouped by
/// exercise the way the page shows them, then the permanent URL.
pub(super) fn share_text(workout: &fitness::Workout, origin: Option<&str>) -> String {
    let card = WorkoutCard::from(workout);
    let volume_points: u32 = card
        .blocks
        .iter()
        .flat_map(|block| block.groups.iter())
        .map(|group| group.volume_points)
        .sum();

    let mut lines = Vec::new();
    lines.push(card.title.to_string());
    lines.push(format!(
        "{} · {} · {} · {} {} · {} volume points",
        card.date,
        card.time_range,
        card.duration,
        card.set_count,
        plural(card.set_count, "set", "sets"),
        format_integer(volume_points),
    ));
    if let Some(description) = card.description {
        lines.push(String::new());
        lines.push(description.to_string());
    }
    if let Some(notes) = card.notes {
        lines.push(String::new());
        lines.push(notes.to_string());
    }

    for block in &card.blocks {
        for group in &block.groups {
            lines.push(String::new());
            match block.superset_id {
                Some(id) => lines.push(format!("{} · superset {id}", group.name)),
                None => lines.push(group.name.to_string()),
            }
            for (index, row) in group.rows.iter().enumerate() {
                let mut line = format!("{}. {}", index + 1, row.prescription);
                if row.set.set_type == "WARMUP_SET" {
                    line.push_str(" · warm-up");
                } else if row.set.set_type == "FAILURE_SET" {
                    line.push_str(" · failure");
                }
                if let Some(effort) = row.set.effort_hundredths {
                    line.push_str(&format!(" @ RPE {}", format_scaled(effort, 100)));
                }
                if !row.details.is_empty() {
                    line.push_str(&format!(" · {}", row.details));
                }
                if let Some(record) = &row.record {
                    line.push_str(&format!(" — {record}"));
                }
                lines.push(line);
            }
        }
    }

    lines.push(String::new());
    match origin {
        Some(origin) => lines.push(format!("{origin}{}", card.href)),
        None => lines.push(card.href.clone()),
    }
    lines.join("\n")
}

/// The disclosure a workout page renders. `text` is prebuilt by the page
/// (it needs the request to know the public origin).
///
/// A readonly `<textarea>` rather than a `<pre>`: the em-dash response
/// layer (`src/emdash.rs`) leaves textarea content alone, so the sheet
/// stays copyable plain text even when a title or note contains an em
/// dash — and it is selectable without JavaScript.
#[component]
pub(super) async fn share_block(text: &str) -> Result {
    let rows = text.lines().count().clamp(3, 14).to_string();
    view! {
        <details class="group" data-share="">
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
                <button class=(SHARE_BUTTON) type="button" data-share-copy="" hidden="">
                    "copy to clipboard"
                </button>
                <p class=(SHARE_HINT) data-share-hint="">
                    "Select the text above to copy it — it already ends with this page's link."
                </p>
            </div>
        </details>
    }
}

/// A share row for the workout-page header list, so the disclosure sits
/// with the date/time/duration facts.
#[component]
pub(super) async fn share_row(text: &str) -> Result {
    view! {
        <div class="rail-row">
            <dt class="rail-stamp rail-stamp-label">"share"</dt>
            <dd class="min-w-0">share_block(text: text)</dd>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(
        ordinal: u32,
        exercise: &str,
        weight: Option<u64>,
        reps: Option<u64>,
        effort: Option<u64>,
        set_type: &str,
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
                ),
                set(
                    2,
                    "Incline Bench Press",
                    Some(145_000),
                    Some(3),
                    Some(800),
                    "NORMAL_SET",
                ),
                set(
                    3,
                    "Cable Crossover",
                    Some(25_000),
                    Some(9),
                    None,
                    "FAILURE_SET",
                ),
            ],
        }
    }

    #[test]
    fn share_text_lists_sets_and_ends_with_the_permalink() {
        let text = share_text(&workout(), Some("https://ben.soy"));
        let expected = "\
I missed 9am gym
Jul 21, 2026 · 10:39 AM–11:14 AM · 35m 10s · 3 sets · 9 volume points

Incline Bench Press
1. 45 lbs × 10 · warm-up
2. 145 lbs × 3 @ RPE 8

Cable Crossover
1. 25 lbs × 9 · failure

https://ben.soy/fitness/lift/2026-07-21T10-39-04-04-00";
        assert_eq!(text, expected);
    }

    #[test]
    fn share_text_without_an_origin_keeps_the_bare_path() {
        let text = share_text(&workout(), None);
        assert!(text.ends_with("\n/fitness/lift/2026-07-21T10-39-04-04-00"));
    }

    #[test]
    fn workout_notes_and_records_ride_along() {
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
        assert!(text.contains("2. 145 lbs × 3 @ RPE 8 — PR: 1RM"));
        // Runner-up podium places never reach the share text.
        assert!(!text.contains('#') && !text.contains("failure —"));
    }
}
