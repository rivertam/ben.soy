//! Shared exercise taxonomy for both fitness ingestion paths.

use std::collections::BTreeSet;

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct ExerciseTag {
    pub(super) kind: String,
    pub(super) value: String,
}

// Deliberately exact: substring matching `squat` would misclassify exercises
// such as "Good Morning (Squat Machine)". These normalized names account for
// exactly 548 rows in the source export audited when this importer was added.
pub(super) const SQUAT_TYPE_EXERCISES: &[&str] = &[
    "Barbell Zercher Squat",
    "Bulgarian Split Squat",
    "Bulgarian Split Squat (Smith Machine)",
    "Deficit Split Squat (Smith Machine)",
    "Dumbbell Assisted Bulgarian Split Squat",
    "Dumbbell Walking Lunges",
    "Full Squat",
    "Lever Horizontal One leg Press",
    "Lever Seated Leg Press",
    "Lunge",
    "Sissy Squat",
    "Sled 45° Leg Press",
    "Sled Hack Squat",
    "Smith Lateral Step-Up",
    "Smith Sprint Lunge",
    "Smith Squat",
    "Step-Up (Weighted)",
    "Step-up",
];

fn has_any(name: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| name.contains(needle))
}

fn has_word(name: &str, word: &str) -> bool {
    name.split(|character: char| !character.is_alphanumeric())
        .any(|candidate| candidate == word)
}

pub(super) fn exercise_tags(name: &str) -> Vec<ExerciseTag> {
    let lower = name.to_lowercase();
    let mut tags: BTreeSet<(&str, &str)> = BTreeSet::new();
    let mut add = |kind, value| {
        tags.insert((kind, value));
    };

    if SQUAT_TYPE_EXERCISES.contains(&name) {
        add("movement", "squat-type");
        add("muscle", "quads");
        add("muscle", "glutes");
    }
    if has_any(
        &lower,
        &[
            "deadlift",
            "good morning",
            "hip thrust",
            "glute bridge",
            "back extension",
            "jefferson curl",
        ],
    ) {
        add("movement", "hinge");
        add("muscle", "hamstrings");
        add("muscle", "glutes");
    }
    if has_any(&lower, &["bench press", "chest press", "push up", "pushup"]) {
        add("movement", "horizontal-push");
        add("muscle", "chest");
        add("muscle", "triceps");
    }
    if has_any(
        &lower,
        &[
            "overhead press",
            "military press",
            "shoulder press",
            "pike push",
        ],
    ) {
        add("movement", "vertical-push");
        add("muscle", "shoulders");
        add("muscle", "triceps");
    }
    if has_word(&lower, "row") {
        add("movement", "horizontal-pull");
        add("muscle", "back");
    }
    if has_any(
        &lower,
        &[
            "pull up",
            "pull-up",
            "chin up",
            "pulldown",
            "vertical traction",
        ],
    ) {
        add("movement", "vertical-pull");
        add("muscle", "back");
    }
    if lower.contains("pullover") {
        add("movement", "shoulder-extension");
        add("muscle", "back");
    }
    if has_any(&lower, &["fly", "crossover", "cross-over", "pec deck"]) {
        add("movement", "fly");
        if !has_any(&lower, &["reverse fly", "rear delt"]) {
            add("muscle", "chest");
        }
    }
    if lower.contains("lateral raise") {
        add("movement", "shoulder-abduction");
        add("muscle", "shoulders");
    }
    if lower.contains("front raise") {
        add("movement", "shoulder-flexion");
        add("muscle", "shoulders");
    }
    if has_any(&lower, &["reverse fly", "rear delt", "face pull"]) {
        add("movement", "rear-delt");
        add("muscle", "shoulders");
    }
    if lower.contains("curl")
        && !lower.contains("leg curl")
        && !lower.contains("jefferson curl")
        && !lower.contains("wrist")
    {
        add("movement", "elbow-flexion");
        add("muscle", "biceps");
    }
    if has_any(&lower, &["triceps", "skull crusher"]) {
        add("movement", "elbow-extension");
        add("muscle", "triceps");
    }
    if lower.contains("dip") {
        add("movement", "dip");
        add("muscle", "triceps");
        if lower.contains("chest") {
            add("muscle", "chest");
        }
    }
    if lower.contains("leg extension") {
        add("movement", "knee-extension");
        add("muscle", "quads");
    }
    if lower.contains("leg curl") {
        add("movement", "knee-flexion");
        add("muscle", "hamstrings");
    }
    if has_any(&lower, &["abductor", "abductors", "glute kickback"]) {
        add("movement", "hip-abduction");
        add("muscle", "glutes");
    }
    if has_any(&lower, &["adductor", "adduction", "inner thigh"]) {
        add("movement", "hip-adduction");
        add("muscle", "adductors");
    }
    if lower.contains("calf raise") {
        add("movement", "calf-raise");
        add("muscle", "calves");
    }
    if lower.contains("shrug") {
        add("movement", "shrug");
        add("muscle", "traps");
    }
    if has_any(
        &lower,
        &[
            "crunch",
            "leg raise",
            "toes to bar",
            "plank",
            "abdominal",
            "torso rotation",
            "russian twist",
            "cable twist",
        ],
    ) {
        add("movement", "core");
        add("muscle", "core");
    }
    if has_any(&lower, &["wrist", "grip roller"]) {
        add("movement", "grip-wrist");
        add("muscle", "forearms");
    }
    if lower.contains("farmer's walk") {
        add("movement", "carry");
    }
    if has_any(&lower, &["running", "stair stepper", "rowing"]) {
        add("movement", "cardio");
    }
    if lower.contains("power clean") {
        add("movement", "olympic-lift");
    }
    if lower.contains("throw") {
        add("movement", "throw");
    }

    if has_any(&lower, &["smith ", "smith machine"]) {
        add("equipment", "smith-machine");
    } else if has_any(
        &lower,
        &[
            "machine",
            "lever ",
            "sled ",
            "mts ",
            "atlantis",
            "roc-it",
            "roc it",
            "booty builder",
            "pec deck",
            "vertical traction",
        ],
    ) {
        add("equipment", "machine");
    }
    if has_any(&lower, &["dumbbell", "dumbbells"]) {
        add("equipment", "dumbbell");
    }
    if has_any(&lower, &["barbell", "ez bar", "ez-bar"]) {
        add("equipment", "barbell");
    }
    if lower.contains("cable") {
        add("equipment", "cable");
    }
    if lower.contains("landmine") || lower.contains("land mine") {
        add("equipment", "landmine");
    }
    if lower.contains("sandbag") {
        add("equipment", "sandbag");
    }
    if lower.contains("medicine ball") {
        add("equipment", "medicine-ball");
    }
    if lower.contains("ring ") {
        add("equipment", "rings");
    }
    let bodyweight_movement = has_any(
        &lower,
        &[
            "pull up",
            "pull-up",
            "chin up",
            "push up",
            "pushup",
            "plank",
            "hanging",
            "bicycle crunch",
            "burpee",
            "bodyweight",
        ],
    ) && !has_any(&lower, &["assisted", "machine"]);
    let unassisted_dip =
        lower.contains("dip") && !has_any(&lower, &["assisted", "lever", "machine"]);
    if bodyweight_movement || unassisted_dip {
        add("equipment", "bodyweight");
    }

    tags.into_iter()
        .map(|(kind, value)| ExerciseTag {
            kind: kind.to_string(),
            value: value.to_string(),
        })
        .collect()
}
