//! Muscle involvement for one workout, derived at render time.
//!
//! The archive stores flat `(kind, value)` exercise tags — there is
//! deliberately no primary/secondary column (`docs/fitness.md`). The
//! split is derived here the same way records are derived at snapshot
//! build: `PRIMARY_BY_MOVEMENT` mirrors which muscles each
//! `taxonomy::exercise_tags` movement rule co-emits as the movers,
//! and any remaining muscle tag on the exercise renders as secondary.
//! Keep that table aligned with the importer's taxonomy rules.

use std::collections::BTreeSet;

use topcoat::{
    Result,
    view::{class, component, view},
};

use super::{
    META_LABEL,
    data::{self as fitness, ExerciseTags},
    filters::MUSCLES,
};

/// The muscles a movement pattern trains as prime movers, out of the
/// muscles the taxonomy tags alongside it. Intersected with each
/// exercise's actual muscle tags, so e.g. `dip` only claims chest on
/// exercises the importer tagged with chest.
const PRIMARY_BY_MOVEMENT: &[(&str, &[&str])] = &[
    ("squat-type", &["quads", "glutes"]),
    ("hinge", &["glutes", "hamstrings"]),
    ("horizontal-push", &["chest"]),
    ("vertical-push", &["shoulders"]),
    ("horizontal-pull", &["back"]),
    ("vertical-pull", &["back"]),
    ("shoulder-extension", &["back"]),
    ("fly", &["chest"]),
    ("shoulder-abduction", &["shoulders"]),
    ("shoulder-flexion", &["shoulders"]),
    ("rear-delt", &["shoulders"]),
    ("elbow-flexion", &["biceps"]),
    ("elbow-extension", &["triceps"]),
    ("dip", &["triceps", "chest"]),
    ("knee-extension", &["quads"]),
    ("knee-flexion", &["hamstrings"]),
    ("hip-abduction", &["glutes"]),
    ("hip-adduction", &["adductors"]),
    ("calf-raise", &["calves"]),
    ("shrug", &["traps"]),
    ("core", &["core"]),
    ("grip-wrist", &["forearms"]),
];

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct MuscleInvolvement {
    /// Canonical muscle ids in `MUSCLES` order.
    pub(super) primary: Vec<&'static str>,
    pub(super) secondary: Vec<&'static str>,
}

impl MuscleInvolvement {
    pub(super) fn is_empty(&self) -> bool {
        self.primary.is_empty() && self.secondary.is_empty()
    }

    fn class_for(&self, muscle: &str) -> &'static str {
        if self.primary.contains(&muscle) {
            DIAGRAM_PRIMARY
        } else if self.secondary.contains(&muscle) {
            DIAGRAM_SECONDARY
        } else {
            DIAGRAM_INACTIVE
        }
    }
}

/// Union of every exercise's emphasis across the workout's sets. A muscle
/// promoted to primary by any exercise stays primary for the workout.
pub(super) fn workout_involvement(
    workout: &fitness::Workout,
    tags: &ExerciseTags,
) -> MuscleInvolvement {
    let mut primary = BTreeSet::new();
    let mut secondary = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for set in &workout.sets {
        if !seen.insert(set.exercise_name.as_str()) {
            continue;
        }
        let Some(pairs) = tags.get(&set.exercise_name) else {
            continue;
        };
        let (exercise_primary, exercise_secondary) = exercise_emphasis(pairs);
        primary.extend(exercise_primary);
        secondary.extend(exercise_secondary);
    }
    MuscleInvolvement {
        primary: canonical_order(&primary),
        secondary: canonical_order(&secondary.difference(&primary).copied().collect()),
    }
}

/// One exercise's split: primaries are its muscle tags claimed by any of
/// its movement tags; the rest of its muscle tags are secondary. An
/// exercise whose movements claim nothing (or that has no movement tag)
/// counts every tagged muscle as primary — with no mover to compare
/// against, "secondary" would be an invention.
fn exercise_emphasis(
    pairs: &[(String, String)],
) -> (BTreeSet<&'static str>, BTreeSet<&'static str>) {
    let muscles: BTreeSet<&'static str> = pairs
        .iter()
        .filter(|(kind, _)| kind == "muscle")
        .filter_map(|(_, value)| canonical_muscle(value))
        .collect();
    let claimed: BTreeSet<&'static str> = pairs
        .iter()
        .filter(|(kind, _)| kind == "movement")
        .flat_map(|(_, value)| {
            PRIMARY_BY_MOVEMENT
                .iter()
                .find(|(movement, _)| movement == value)
                .map(|(_, movers)| movers.iter().copied())
                .into_iter()
                .flatten()
        })
        .collect();
    let primary: BTreeSet<&'static str> = muscles.intersection(&claimed).copied().collect();
    if primary.is_empty() {
        return (muscles, BTreeSet::new());
    }
    let secondary = muscles.difference(&primary).copied().collect();
    (primary, secondary)
}

/// Map a stored tag value onto the site's canonical muscle vocabulary.
fn canonical_muscle(value: &str) -> Option<&'static str> {
    MUSCLES
        .iter()
        .find_map(|(id, _)| (*id == value).then_some(*id))
}

fn canonical_order(muscles: &BTreeSet<&'static str>) -> Vec<&'static str> {
    MUSCLES
        .iter()
        .filter_map(|(id, _)| muscles.contains(id).then_some(*id))
        .collect()
}

fn label_list(muscles: &[&'static str]) -> String {
    muscles
        .iter()
        .filter_map(|id| super::filters::lookup(MUSCLES, id))
        .collect::<Vec<_>>()
        .join(" · ")
}

// Tailwind vocab for the diagram. Utilities stay whole per line for the
// build-time class scanner.
const DIAGRAM_SILHOUETTE: &str = "fill-ink/4 stroke-hairline";
const DIAGRAM_INACTIVE: &str = "fill-ink/8 stroke-ink/14";
const DIAGRAM_PRIMARY: &str = "fill-oxide/85 stroke-oxide";
const DIAGRAM_SECONDARY: &str = "fill-oxide/30 stroke-oxide/55";
const FIGURE_CAPTION: &str =
    "mt-2 text-center font-meta text-[0.61rem] leading-none tracking-[0.13em] uppercase text-muted";
const LEGEND_ROW: &str = "flex items-center gap-[0.45rem] font-meta text-[0.7rem] text-ink2";
const LEGEND_SWATCH_BASE: &str = "inline-block size-[0.7rem] flex-none rounded-[0.12rem] border";
const LEGEND_SWATCH_PRIMARY: &str = "bg-oxide/85 border-oxide";
const LEGEND_SWATCH_SECONDARY: &str = "bg-oxide/30 border-oxide/55";

struct MusclePath {
    muscle: &'static str,
    d: &'static str,
}

/// Both figures share one stylized silhouette; muscle regions are drawn
/// per side so left/right shade together.
const SILHOUETTE: &str = "M100 8 C 91 8 85 15 85 24 C 85 31 87 36 91 40 C 91 44 90 47 86 49 C 74 53 64 58 60 66 C 55 76 53 88 53 98 C 53 110 51 122 48 132 C 45 142 43 154 43 166 C 43 175 45 182 47 187 L 58 184 C 60 176 61 168 62 160 C 64 172 64 184 63 194 C 62 210 63 228 66 244 C 69 260 71 276 70 292 C 69 308 68 322 69 334 C 70 346 72 356 74 362 L 90 362 C 91 354 91 344 90 332 C 89 318 90 304 92 292 C 94 278 96 262 97 248 L 100 232 L 103 248 C 104 262 106 278 108 292 C 110 304 111 318 110 332 C 109 344 109 354 110 362 L 126 362 C 128 356 130 346 131 334 C 132 322 131 308 130 292 C 129 276 131 260 134 244 C 137 228 138 210 137 194 C 136 184 136 172 138 160 C 139 168 140 176 142 184 L 153 187 C 155 182 157 175 157 166 C 157 154 155 142 152 132 C 149 122 147 110 147 98 C 147 88 145 76 140 66 C 136 58 126 53 114 49 C 110 47 109 44 109 40 C 113 36 115 31 115 24 C 115 15 109 8 100 8 Z";

const FRONT_PATHS: &[MusclePath] = &[
    MusclePath {
        muscle: "traps",
        d: "M86 50 C 78 53 71 56 67 60 C 76 58 85 57 92 57 C 91 54 88 52 86 50 Z",
    },
    MusclePath {
        muscle: "traps",
        d: "M114 50 C 122 53 129 56 133 60 C 124 58 115 57 108 57 C 109 54 112 52 114 50 Z",
    },
    MusclePath {
        muscle: "shoulders",
        d: "M60 66 C 66 61 73 60 78 62 C 74 70 71 80 70 89 C 65 87 60 83 57 78 C 57 73 58 69 60 66 Z",
    },
    MusclePath {
        muscle: "shoulders",
        d: "M140 66 C 134 61 127 60 122 62 C 126 70 129 80 130 89 C 135 87 140 83 143 78 C 143 73 142 69 140 66 Z",
    },
    MusclePath {
        muscle: "chest",
        d: "M80 63 C 88 60 96 60 99 61 L 99 92 C 92 96 83 95 77 90 C 73 86 72 76 74 69 C 75 66 77 64 80 63 Z",
    },
    MusclePath {
        muscle: "chest",
        d: "M120 63 C 112 60 104 60 101 61 L 101 92 C 108 96 117 95 123 90 C 127 86 128 76 126 69 C 125 66 123 64 120 63 Z",
    },
    MusclePath {
        muscle: "biceps",
        d: "M58 92 C 62 96 67 99 70 100 C 70 112 68 122 65 130 C 61 129 57 126 55 122 C 54 112 55 100 58 92 Z",
    },
    MusclePath {
        muscle: "biceps",
        d: "M142 92 C 138 96 133 99 130 100 C 130 112 132 122 135 130 C 139 129 143 126 145 122 C 146 112 145 100 142 92 Z",
    },
    MusclePath {
        muscle: "forearms",
        d: "M54 130 C 57 134 61 136 64 137 C 62 150 59 162 56 172 C 53 174 50 175 47 175 C 46 165 48 148 54 130 Z",
    },
    MusclePath {
        muscle: "forearms",
        d: "M146 130 C 143 134 139 136 136 137 C 138 150 141 162 144 172 C 147 174 150 175 153 175 C 154 165 152 148 146 130 Z",
    },
    MusclePath {
        muscle: "core",
        d: "M84 98 C 89 101 95 102 100 102 C 105 102 111 101 116 98 C 119 110 120 124 118 138 C 114 152 108 162 100 168 C 92 162 86 152 82 138 C 80 124 81 110 84 98 Z",
    },
    MusclePath {
        muscle: "quads",
        d: "M66 196 C 72 190 80 186 88 185 C 92 196 94 210 94 224 C 94 240 91 254 86 264 C 80 266 73 265 69 261 C 65 244 64 218 66 196 Z",
    },
    MusclePath {
        muscle: "quads",
        d: "M134 196 C 128 190 120 186 112 185 C 108 196 106 210 106 224 C 106 240 109 254 114 264 C 120 266 127 265 131 261 C 135 244 136 218 134 196 Z",
    },
    MusclePath {
        muscle: "adductors",
        d: "M97 186 L 97 226 C 93 218 91 202 91 191 C 93 188 95 187 97 186 Z",
    },
    MusclePath {
        muscle: "adductors",
        d: "M103 186 L 103 226 C 107 218 109 202 109 191 C 107 188 105 187 103 186 Z",
    },
    MusclePath {
        muscle: "calves",
        d: "M73 290 C 77 286 82 284 86 285 C 87 298 86 312 83 324 C 80 325 76 324 74 321 C 72 312 72 300 73 290 Z",
    },
    MusclePath {
        muscle: "calves",
        d: "M127 290 C 123 286 118 284 114 285 C 113 298 114 312 117 324 C 120 325 124 324 126 321 C 128 312 128 300 127 290 Z",
    },
];

const BACK_PATHS: &[MusclePath] = &[
    MusclePath {
        muscle: "traps",
        d: "M100 48 C 94 52 88 56 78 60 C 86 62 93 66 96 72 C 98 80 99 90 100 98 C 101 90 102 80 104 72 C 107 66 114 62 122 60 C 112 56 106 52 100 48 Z",
    },
    MusclePath {
        muscle: "shoulders",
        d: "M60 66 C 66 61 73 60 78 62 C 74 70 71 80 70 89 C 65 87 60 83 57 78 C 57 73 58 69 60 66 Z",
    },
    MusclePath {
        muscle: "shoulders",
        d: "M140 66 C 134 61 127 60 122 62 C 126 70 129 80 130 89 C 135 87 140 83 143 78 C 143 73 142 69 140 66 Z",
    },
    MusclePath {
        muscle: "back",
        d: "M78 64 C 83 70 88 80 92 90 C 95 104 97 122 97 138 C 90 134 82 126 77 116 C 73 106 72 88 74 74 C 75 70 76 66 78 64 Z",
    },
    MusclePath {
        muscle: "back",
        d: "M122 64 C 117 70 112 80 108 90 C 105 104 103 122 103 138 C 110 134 118 126 123 116 C 127 106 128 88 126 74 C 125 70 124 66 122 64 Z",
    },
    MusclePath {
        muscle: "back",
        d: "M96 104 L 104 104 C 105 122 105 140 104 156 L 96 156 C 95 140 95 122 96 104 Z",
    },
    MusclePath {
        muscle: "triceps",
        d: "M58 92 C 62 96 67 99 70 100 C 70 112 68 122 65 130 C 61 129 57 126 55 122 C 54 112 55 100 58 92 Z",
    },
    MusclePath {
        muscle: "triceps",
        d: "M142 92 C 138 96 133 99 130 100 C 130 112 132 122 135 130 C 139 129 143 126 145 122 C 146 112 145 100 142 92 Z",
    },
    MusclePath {
        muscle: "forearms",
        d: "M54 130 C 57 134 61 136 64 137 C 62 150 59 162 56 172 C 53 174 50 175 47 175 C 46 165 48 148 54 130 Z",
    },
    MusclePath {
        muscle: "forearms",
        d: "M146 130 C 143 134 139 136 136 137 C 138 150 141 162 144 172 C 147 174 150 175 153 175 C 154 165 152 148 146 130 Z",
    },
    MusclePath {
        muscle: "glutes",
        d: "M80 160 C 86 158 93 158 98 160 C 100 170 100 180 98 188 C 92 194 83 194 77 188 C 74 180 75 168 80 160 Z",
    },
    MusclePath {
        muscle: "glutes",
        d: "M120 160 C 114 158 107 158 102 160 C 100 170 100 180 102 188 C 108 194 117 194 123 188 C 126 180 125 168 120 160 Z",
    },
    MusclePath {
        muscle: "hamstrings",
        d: "M69 200 C 75 197 83 196 90 198 C 93 212 93 230 90 246 C 87 258 82 266 76 268 C 71 260 67 244 67 226 C 67 216 68 207 69 200 Z",
    },
    MusclePath {
        muscle: "hamstrings",
        d: "M131 200 C 125 197 117 196 110 198 C 107 212 107 230 110 246 C 113 258 118 266 124 268 C 129 260 133 244 133 226 C 133 216 132 207 131 200 Z",
    },
    MusclePath {
        muscle: "calves",
        d: "M72 288 C 77 282 84 280 89 282 C 91 296 90 312 86 326 C 82 328 77 327 74 323 C 71 312 70 298 72 288 Z",
    },
    MusclePath {
        muscle: "calves",
        d: "M128 288 C 123 282 116 280 111 282 C 109 296 110 312 114 326 C 118 328 123 327 126 323 C 129 312 130 298 128 288 Z",
    },
];

/// The muscle map a workout page shows: front/back figures plus the
/// primary/secondary lists as text. The text is the accessible content;
/// the figures are decorative duplicates of it, so they stay aria-hidden
/// rather than each re-announcing the workout-wide lists.
#[component]
pub(super) async fn muscle_map(involvement: &MuscleInvolvement) -> Result {
    let primary_list = label_list(&involvement.primary);
    let secondary_list = label_list(&involvement.secondary);
    view! {
        <section aria-label="Muscles worked">
            if involvement.is_empty() {
                <p class="max-w-[32rem] font-meta text-[0.72rem] leading-[1.55] text-muted">
                    "These exercises are not in the muscle taxonomy yet, so there is no map to draw."
                </p>
            } else {
                // At the workout page's gutter breakpoint the map lives in
                // a 14.5rem side panel, so the figures shrink to sit two-up
                // with the lists wrapping beneath them.
                <div class="flex flex-wrap items-start gap-x-8 gap-y-5 min-[90rem]:gap-x-4">
                    muscle_figure(
                        paths: FRONT_PATHS,
                        caption: "front",
                        involvement: involvement
                    )
                    muscle_figure(
                        paths: BACK_PATHS,
                        caption: "back",
                        involvement: involvement
                    )
                    <dl class="min-w-[11rem] flex-1 space-y-3 sm:pt-2">
                        if !involvement.primary.is_empty() {
                            <div>
                                <dt class=(class!(META_LABEL, "flex items-center gap-[0.45rem]"))>
                                    <span
                                        class=(class!(LEGEND_SWATCH_BASE, LEGEND_SWATCH_PRIMARY))
                                        aria-hidden="true"
                                    >
                                    </span>
                                    "primary"
                                </dt>
                                <dd class=(class!(LEGEND_ROW, "mt-[0.35rem]"))>
                                    (primary_list.as_str())
                                </dd>
                            </div>
                        }
                        if !involvement.secondary.is_empty() {
                            <div>
                                <dt class=(class!(META_LABEL, "flex items-center gap-[0.45rem]"))>
                                    <span
                                        class=(class!(LEGEND_SWATCH_BASE, LEGEND_SWATCH_SECONDARY))
                                        aria-hidden="true"
                                    >
                                    </span>
                                    "secondary"
                                </dt>
                                <dd class=(class!(LEGEND_ROW, "mt-[0.35rem]"))>
                                    (secondary_list.as_str())
                                </dd>
                            </div>
                        }
                    </dl>
                </div>
            }
        </section>
    }
}

#[component]
async fn muscle_figure(
    paths: &'static [MusclePath],
    caption: &str,
    involvement: &MuscleInvolvement,
) -> Result {
    view! {
        <figure
            class="m-0 w-[8.5rem] flex-none sm:w-[9.5rem] min-[90rem]:w-[6.6rem]"
            aria-hidden="true"
        >
            <svg viewBox="0 0 200 380">
                <path class=(DIAGRAM_SILHOUETTE) stroke-width="1.5" d=(SILHOUETTE)></path>
                for path in paths.iter() {
                    <path
                        class=(involvement.class_for(path.muscle))
                        stroke-width="0.75"
                        d=(path.d)
                    ></path>
                }
            </svg>
            <figcaption class=(FIGURE_CAPTION)>(caption)</figcaption>
        </figure>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(kind: &str, value: &str) -> (String, String) {
        (kind.to_string(), value.to_string())
    }

    fn workout_with(exercises: &[&str]) -> fitness::Workout {
        fitness::Workout {
            id: "w".into(),
            path: "2026-07-21T10-39-04-04-00".into(),
            title: "Push day".into(),
            raw_title: "Push day".into(),
            started_at_local: "2026-07-21 10:39:04".into(),
            ended_at_local: "2026-07-21 11:14:14".into(),
            eastern_offset_minutes: -240,
            end_eastern_offset_minutes: -240,
            duration_seconds: 2110,
            duration_suspicious: false,
            notes: None,
            description: None,
            sets: exercises
                .iter()
                .enumerate()
                .map(|(index, name)| fitness::Set {
                    id: format!("s{index}"),
                    ordinal: index as u32 + 1,
                    exercise_name: (*name).to_string(),
                    raw_exercise_name: (*name).to_string(),
                    exercise_note: None,
                    superset_id: None,
                    weight_milli: Some(100_000),
                    weight_unit: "lbs".into(),
                    reps: Some(5),
                    effort_hundredths: None,
                    distance_milli: None,
                    set_time_seconds: None,
                    set_type: "NORMAL_SET".into(),
                    records: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn movement_rules_split_primary_from_secondary() {
        let (primary, secondary) = exercise_emphasis(&[
            tag("equipment", "barbell"),
            tag("movement", "horizontal-push"),
            tag("muscle", "chest"),
            tag("muscle", "triceps"),
        ]);
        assert_eq!(primary.into_iter().collect::<Vec<_>>(), vec!["chest"]);
        assert_eq!(secondary.into_iter().collect::<Vec<_>>(), vec!["triceps"]);

        // Hinge claims both of its co-emitted muscles as movers.
        let (primary, secondary) = exercise_emphasis(&[
            tag("movement", "hinge"),
            tag("muscle", "glutes"),
            tag("muscle", "hamstrings"),
        ]);
        assert_eq!(primary.len(), 2);
        assert!(secondary.is_empty());
    }

    #[test]
    fn unclaimed_muscles_default_to_primary() {
        // No movement tag at all: the muscle list is the whole story.
        let (primary, secondary) = exercise_emphasis(&[tag("muscle", "chest")]);
        assert_eq!(primary.into_iter().collect::<Vec<_>>(), vec!["chest"]);
        assert!(secondary.is_empty());

        // A movement the table does not claim muscles for (e.g. carry)
        // behaves the same way.
        let (primary, secondary) =
            exercise_emphasis(&[tag("movement", "carry"), tag("muscle", "forearms")]);
        assert_eq!(primary.into_iter().collect::<Vec<_>>(), vec!["forearms"]);
        assert!(secondary.is_empty());
    }

    #[test]
    fn workout_union_promotes_primary_over_secondary() {
        let workout = workout_with(&["Overhead Press (Barbell)", "Skull Crusher", "Leg Press"]);
        let mut tags = ExerciseTags::new();
        tags.insert(
            "Overhead Press (Barbell)".into(),
            vec![
                tag("movement", "vertical-push"),
                tag("muscle", "shoulders"),
                tag("muscle", "triceps"),
            ],
        );
        tags.insert(
            "Skull Crusher".into(),
            vec![tag("movement", "elbow-extension"), tag("muscle", "triceps")],
        );
        // "Leg Press" stays untagged, mirroring the real taxonomy gap.

        let involvement = workout_involvement(&workout, &tags);
        assert_eq!(
            involvement.primary,
            vec!["shoulders", "triceps"],
            "triceps is secondary for the press but primary for the extension"
        );
        assert!(involvement.secondary.is_empty());
    }

    #[test]
    fn output_follows_the_site_muscle_order_and_empty_reports_empty() {
        let workout = workout_with(&["Deadlift (Barbell)", "Bench Press (Barbell)"]);
        let mut tags = ExerciseTags::new();
        tags.insert(
            "Deadlift (Barbell)".into(),
            vec![
                tag("movement", "hinge"),
                tag("muscle", "hamstrings"),
                tag("muscle", "glutes"),
            ],
        );
        tags.insert(
            "Bench Press (Barbell)".into(),
            vec![
                tag("movement", "horizontal-push"),
                tag("muscle", "triceps"),
                tag("muscle", "chest"),
            ],
        );
        let involvement = workout_involvement(&workout, &tags);
        assert_eq!(
            involvement.primary,
            vec!["glutes", "hamstrings", "chest"],
            "MUSCLES order, not alphabetical"
        );
        assert_eq!(involvement.secondary, vec!["triceps"]);
        assert!(!involvement.is_empty());

        let untagged = workout_involvement(&workout_with(&["Leg Press"]), &ExerciseTags::new());
        assert!(untagged.is_empty());
    }

    #[test]
    fn every_diagram_path_and_movement_rule_uses_canonical_ids() {
        for (movement, movers) in PRIMARY_BY_MOVEMENT {
            assert!(
                super::super::filters::MOVEMENTS
                    .iter()
                    .chain(super::super::filters::MOVEMENT_DETAILS)
                    .any(|(id, _)| id == movement),
                "unknown movement {movement}"
            );
            for mover in *movers {
                assert!(
                    canonical_muscle(mover).is_some(),
                    "unknown muscle {mover} for {movement}"
                );
            }
        }
        for path in FRONT_PATHS.iter().chain(BACK_PATHS) {
            assert!(
                canonical_muscle(path.muscle).is_some(),
                "unknown muscle {} in diagram",
                path.muscle
            );
        }
        // Every site muscle appears somewhere on the two figures.
        for (muscle, _) in MUSCLES {
            assert!(
                FRONT_PATHS
                    .iter()
                    .chain(BACK_PATHS)
                    .any(|path| path.muscle == *muscle),
                "muscle {muscle} missing from the diagram"
            );
        }
    }
}
