//! The same compact workout in the activity log and heatmap day preview.
//! Grouping, set numbers, and effort seals match the full lift and social PNG.

use topcoat::{
    Result,
    view::{class, component, view},
};

use super::{
    badge, data as fitness, exercise,
    format::plural,
    muscles::{self, MuscleInvolvement},
    results::{SetRow, WorkoutCard},
};

#[component]
pub(super) async fn workout_log_entry(
    workout: &fitness::Workout,
    weights: &fitness::ExerciseWeights,
) -> Result {
    let card = WorkoutCard::from(workout);
    let involvement = muscles::involvement_for_exercises(
        workout.sets.iter().map(|set| set.exercise_name.as_str()),
        weights,
    );
    view! {
        <article class="rail-row rail-row-top">
            <div class="rail-stamp sm:pt-3">
                <time class="flex flex-col gap-1" datetime=(card.datetime.as_str())>
                    <span class="text-ink2">(card.date.as_str())</span>
                    <span class="text-[0.68rem] text-muted">(card.time_range.as_str())</span>
                </time>
            </div>
            workout_summary(workout: workout, involvement: &involvement)
        </article>
    }
}

#[component]
pub(super) async fn workout_summary(
    workout: &fitness::Workout,
    involvement: &MuscleInvolvement,
    #[default(false)] preview: bool,
) -> Result {
    let card = WorkoutCard::from(workout);
    let title_label = format!("Open {} workout", card.title);
    // A preview can show a workout already present in the log. Its nested set
    // popovers need separate identities, including when their ordinals match.
    let scope = if preview { "preview" } else { "log" };
    view! {
        <div class=(class!("lift-summary", "lift-summary-preview" if preview))>
            <header class="lift-summary-header">
                <h3>
                    <a href=(card.href.as_str()) aria-label=(title_label.as_str())>
                        (card.title)
                    </a>
                </h3>
                <p class="lift-summary-facts">
                    if preview {
                        <time datetime=(card.datetime.as_str())>(card.time_range.as_str())</time>
                        " · "
                    }
                    (format!("{} · {} {}", card.duration, card.working_set_count,
                        plural(card.working_set_count, "set", "sets")))
                    if card.duration_suspicious {
                        <span class="text-oxide" title="The source timer recorded zero or at least four hours.">
                            " · timer outlier"
                        </span>
                    }
                </p>
            </header>
            <div class=(class!("lift-summary-body", "lift-summary-with-muscles" if !involvement.is_empty()))>
                <div class="lift-summary-exercises">
                    for block in &card.blocks {
                        <div class=(class!("lift-summary-block", "lift-summary-superset" if block.superset_id.is_some()))>
                            if let Some(id) = block.superset_id {
                                <p class="lift-summary-superset-label">(format!("Superset {id}"))</p>
                            }
                            for group in &block.groups {
                                <section class="lift-summary-exercise">
                                    <h4><a href=(exercise::page_url(group.name))>(group.name)</a></h4>
                                    <ol class="lift-summary-sets" aria-label=(format!("{} sets", group.name))>
                                        for row in &group.rows {
                                            compact_set(row: row, scope: scope)
                                        }
                                    </ol>
                                </section>
                            }
                        </div>
                    }
                </div>
                if !involvement.is_empty() {
                    <aside class="lift-summary-muscles" aria-label="Muscle involvement">
                        muscles::muscle_map_compact(involvement: involvement)
                    </aside>
                }
            </div>
        </div>
    }
}

#[component]
async fn compact_set(row: &SetRow<'_>, scope: &str) -> Result {
    let id = format!("{scope}-{}", row.effort_popover_id);
    view! {
        <li>
            badge::compact_set_badge(row: row, popover_id: id.as_str())
        </li>
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[tokio::test]
    async fn log_and_day_preview_keep_every_set_with_separate_popover_targets() {
        let mut fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/api/latest.json"
        )))
        .unwrap();
        // Bring this older golden fixture onto the current failure wire shape.
        for set in fixture["workout"]["sets"].as_array_mut().unwrap() {
            let failure = set["set_type"] == "FAILURE_SET";
            set["failure"] = failure.into();
            if failure {
                set["set_type"] = "NORMAL_SET".into();
                set["effort_hundredths"] = serde_json::Value::Null;
            }
        }
        let mut workout: fitness::Workout =
            serde_json::from_value(fixture["workout"].take()).unwrap();
        workout.title = "Lift <script>alert(1)</script>".into();
        for (index, set) in workout.sets.iter_mut().enumerate() {
            set.exercise_name = format!("Exercise {}", index / 3 + 1);
        }
        workout.sets[0].set_type = "WARMUP_SET".into();
        workout.sets[1].set_type = "NORMAL_SET".into();
        workout.sets[1].failure = false;
        workout.sets[1].weight_milli = Some(-45_500);
        workout.sets[1].reps = Some(8);
        workout.sets[1].effort_hundredths = Some(950);
        workout.sets[1].records = vec![fitness::Record {
            level: "gold".into(),
            kind: "reps".into(),
        }];
        workout.sets[1].superset_id = Some(7);
        let involvement = MuscleInvolvement::default();
        let cx = topcoat::context::Cx::default();
        let __cx = &cx;
        let html = view! {
            workout_summary(workout: &workout, involvement: &involvement)
            workout_summary(workout: &workout, involvement: &involvement, preview: true)
        }
        .unwrap()
        .render(__cx);
        assert_eq!(
            html.matches("class=\"lift-set-compact\"").count(),
            workout.sets.len() * 2
        );
        let card = WorkoutCard::from(&workout);
        let working_sets = workout
            .sets
            .iter()
            .filter(|set| set.set_type != "WARMUP_SET")
            .count();
        assert!(working_sets < workout.sets.len());
        assert_eq!(card.working_set_count, working_sets);
        assert!(html.contains(&format!(
            " · {} {}",
            card.working_set_count,
            plural(card.working_set_count, "set", "sets"),
        )));
        let groups: usize = card.blocks.iter().map(|block| block.groups.len()).sum();
        assert_eq!(html.matches("<h4>").count(), groups * 2);
        assert!(html.contains("Exercise 5"));
        assert!(html.contains("-45.5 lbs × 8"));
        assert!(html.contains("0.5 RIR"));
        assert!(html.contains("PR: reps"));
        assert!(html.contains("Superset 7"));
        assert!(html.contains("Warm-up set"));
        // Angle brackets are legal inside a quoted aria-label; text content
        // must still be escaped rather than becoming a script element.
        assert!(html.contains(">Lift &lt;script&gt;alert(1)&lt;/script&gt;</a>"));
        let ids: Vec<_> = html
            .split(" id=\"")
            .skip(1)
            .map(|part| part.split('"').next().unwrap())
            .collect();
        let unique: HashSet<_> = ids.iter().copied().collect();
        assert_eq!(ids.len(), workout.sets.len() * 2);
        assert_eq!(
            ids.len(),
            unique.len(),
            "Log and preview IDs must never collide"
        );
        for target in html
            .split(" popovertarget=\"")
            .skip(1)
            .map(|part| part.split('"').next().unwrap())
        {
            assert!(unique.contains(target), "Missing popover {target}");
        }
    }
}
