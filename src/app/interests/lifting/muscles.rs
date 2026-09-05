//! Muscle involvement for one workout, derived at render time.
//!
//! The archive stores weighted exercise↔muscle connections
//! (`exercise_muscles`, ratio in hundredths); the body map derives its
//! shading from those ratios alone: at or above [`PRIMARY_THRESHOLD`] a
//! muscle shades as primary, any smaller stored ratio as secondary, and a
//! muscle with no row stays inactive. Nothing here is stored — like
//! records, the split recomputes whenever the ratios change.

use std::collections::{BTreeSet, HashMap};

use topcoat::{
    Result,
    view::{class, component, view},
};

use super::{META_LABEL, archive::api as fitness, muscle_taxonomy};

/// `(granular muscle id, ratio_hundredths)` per exercise, from
/// `Snapshot::exercise_weight_map`.
type ExerciseWeights = HashMap<String, Vec<(&'static str, u32)>>;

/// Ratio at which a muscle shades as a prime mover. Seed defaults put
/// movers at 100 and synergists at 50, so this reproduces the pre-weight
/// primary/secondary shading; it is purely a display constant.
pub(super) const PRIMARY_THRESHOLD: u32 = 75;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct MuscleInvolvement {
    /// Canonical granular muscle ids in taxonomy display order.
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
    weights: &ExerciseWeights,
) -> MuscleInvolvement {
    involvement_for_exercises(
        workout.sets.iter().map(|set| set.exercise_name.as_str()),
        weights,
    )
}

/// Same split as [`workout_involvement`], from an already-deduped or
/// raw exercise-name stream (duplicates are ignored).
pub(super) fn involvement_for_exercises<'a>(
    exercises: impl IntoIterator<Item = &'a str>,
    weights: &ExerciseWeights,
) -> MuscleInvolvement {
    let mut primary = BTreeSet::new();
    let mut secondary = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for name in exercises {
        if !seen.insert(name) {
            continue;
        }
        let Some(pairs) = weights.get(name) else {
            continue;
        };
        for (muscle, ratio) in pairs {
            if *ratio >= PRIMARY_THRESHOLD {
                primary.insert(*muscle);
            } else {
                secondary.insert(*muscle);
            }
        }
    }
    MuscleInvolvement {
        primary: canonical_order(&primary),
        secondary: canonical_order(&secondary.difference(&primary).copied().collect()),
    }
}

fn canonical_order(muscles: &BTreeSet<&'static str>) -> Vec<&'static str> {
    let mut ordered: Vec<&'static str> = muscles.iter().copied().collect();
    ordered.sort_unstable_by_key(|muscle| muscle_taxonomy::muscle_order(muscle));
    ordered
}

fn label_list(muscles: &[&'static str]) -> String {
    muscles
        .iter()
        .filter_map(|id| muscle_taxonomy::muscle_label(id))
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

pub(super) struct MusclePath {
    muscle: &'static str,
    d: &'static str,
}

/// Both figures share one stylized silhouette; muscle regions are drawn
/// per side so left/right shade together. Regions are granular — delt
/// heads, traps thirds, chest bands — and overlapping neighbors are drawn
/// later so they sit on top.
const SILHOUETTE: &str = "M100 8 C 91 8 85 15 85 24 C 85 31 87 36 91 40 C 91 44 90 47 86 49 C 74 53 64 58 60 66 C 55 76 53 88 53 98 C 53 110 51 122 48 132 C 45 142 43 154 43 166 C 43 175 45 182 47 187 L 58 184 C 60 176 61 168 62 160 C 64 172 64 184 63 194 C 62 210 63 228 66 244 C 69 260 71 276 70 292 C 69 308 68 322 69 334 C 70 346 72 356 74 362 L 90 362 C 91 354 91 344 90 332 C 89 318 90 304 92 292 C 94 278 96 262 97 248 L 100 232 L 103 248 C 104 262 106 278 108 292 C 110 304 111 318 110 332 C 109 344 109 354 110 362 L 126 362 C 128 356 130 346 131 334 C 132 322 131 308 130 292 C 129 276 131 260 134 244 C 137 228 138 210 137 194 C 136 184 136 172 138 160 C 139 168 140 176 142 184 L 153 187 C 155 182 157 175 157 166 C 157 154 155 142 152 132 C 149 122 147 110 147 98 C 147 88 145 76 140 66 C 136 58 126 53 114 49 C 110 47 109 44 109 40 C 113 36 115 31 115 24 C 115 15 109 8 100 8 Z";

pub(super) const FRONT_PATHS: &[MusclePath] = &[
    MusclePath {
        muscle: "upper-traps",
        d: "M86 50 C 78 53 71 56 67 60 C 76 58 85 57 92 57 C 91 54 88 52 86 50 Z",
    },
    MusclePath {
        muscle: "upper-traps",
        d: "M114 50 C 122 53 129 56 133 60 C 124 58 115 57 108 57 C 109 54 112 52 114 50 Z",
    },
    MusclePath {
        muscle: "lateral-delts",
        d: "M60 66 C 63 63 68 61 72 61 C 69 67 67 75 66 83 C 62 81 59 80 57 78 C 57 73 58 69 60 66 Z",
    },
    MusclePath {
        muscle: "lateral-delts",
        d: "M140 66 C 137 63 132 61 128 61 C 131 67 133 75 134 83 C 138 81 141 80 143 78 C 143 73 142 69 140 66 Z",
    },
    MusclePath {
        muscle: "anterior-delts",
        d: "M78 62 C 74 70 71 80 70 89 C 68 88 67 86 66 84 C 67 74 70 66 73 61 C 75 61 77 61 78 62 Z",
    },
    MusclePath {
        muscle: "anterior-delts",
        d: "M122 62 C 126 70 129 80 130 89 C 132 88 133 86 134 84 C 133 74 130 66 127 61 C 125 61 123 61 122 62 Z",
    },
    MusclePath {
        muscle: "upper-chest",
        d: "M80 63 C 88 60 96 60 99 61 L 99 72 L 74 72 C 74 71 74 70 74 69 C 75 66 77 64 80 63 Z",
    },
    MusclePath {
        muscle: "upper-chest",
        d: "M120 63 C 112 60 104 60 101 61 L 101 72 L 126 72 C 126 71 126 70 126 69 C 125 66 123 64 120 63 Z",
    },
    MusclePath {
        muscle: "mid-chest",
        d: "M74 72 L 99 72 L 99 84 C 92 86 80 86 74 84 C 73 80 73 76 74 72 Z",
    },
    MusclePath {
        muscle: "mid-chest",
        d: "M126 72 L 101 72 L 101 84 C 108 86 120 86 126 84 C 127 80 127 76 126 72 Z",
    },
    MusclePath {
        muscle: "lower-chest",
        d: "M74 84 C 80 86 92 86 99 84 L 99 92 C 92 96 83 95 77 90 C 75 88 74 86 74 84 Z",
    },
    MusclePath {
        muscle: "lower-chest",
        d: "M126 84 C 120 86 108 86 101 84 L 101 92 C 108 96 117 95 123 90 C 125 88 126 86 126 84 Z",
    },
    MusclePath {
        muscle: "serratus-anterior",
        d: "M75 92 C 78 95 81 96 83 97 C 82 103 80 108 78 111 C 76 107 74 99 75 92 Z",
    },
    MusclePath {
        muscle: "serratus-anterior",
        d: "M125 92 C 122 95 119 96 117 97 C 118 103 120 108 122 111 C 124 107 126 99 125 92 Z",
    },
    MusclePath {
        muscle: "biceps",
        d: "M58 92 C 62 96 67 99 70 100 C 70 110 69 118 67 124 C 63 123 59 120 56 116 C 55 108 56 98 58 92 Z",
    },
    MusclePath {
        muscle: "biceps",
        d: "M142 92 C 138 96 133 99 130 100 C 130 110 131 118 133 124 C 137 123 141 120 144 116 C 145 108 144 98 142 92 Z",
    },
    MusclePath {
        muscle: "brachialis",
        d: "M56 118 C 59 122 63 124 66 125 C 66 127 65 129 65 130 C 61 129 57 126 55 122 C 55 121 56 119 56 118 Z",
    },
    MusclePath {
        muscle: "brachialis",
        d: "M144 118 C 141 122 137 124 134 125 C 134 127 135 129 135 130 C 139 129 143 126 145 122 C 145 121 144 119 144 118 Z",
    },
    MusclePath {
        muscle: "forearm-flexors",
        d: "M54 130 C 57 134 61 136 64 137 C 62 150 59 162 56 172 C 53 174 50 175 47 175 C 46 165 48 148 54 130 Z",
    },
    MusclePath {
        muscle: "forearm-flexors",
        d: "M146 130 C 143 134 139 136 136 137 C 138 150 141 162 144 172 C 147 174 150 175 153 175 C 154 165 152 148 146 130 Z",
    },
    MusclePath {
        muscle: "obliques",
        d: "M84 98 C 86 99 88 100 90 101 C 88 114 88 132 90 145 C 87 143 84 140 82 136 C 80 123 81 109 84 98 Z",
    },
    MusclePath {
        muscle: "obliques",
        d: "M116 98 C 114 99 112 100 110 101 C 112 114 112 132 110 145 C 113 143 116 140 118 136 C 120 123 119 109 116 98 Z",
    },
    MusclePath {
        muscle: "abs",
        d: "M90 100 C 93 101 97 102 100 102 C 103 102 107 101 110 100 C 112 112 112 130 110 144 C 107 156 104 163 100 167 C 96 163 93 156 90 144 C 88 130 88 112 90 100 Z",
    },
    MusclePath {
        muscle: "hip-flexors",
        d: "M88 168 C 92 172 96 176 98 180 C 96 184 93 186 90 187 C 87 181 86 174 88 168 Z",
    },
    MusclePath {
        muscle: "hip-flexors",
        d: "M112 168 C 108 172 104 176 102 180 C 104 184 107 186 110 187 C 113 181 114 174 112 168 Z",
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
        muscle: "gastrocnemius",
        d: "M73 290 C 77 286 82 284 86 285 C 87 298 86 312 83 324 C 80 325 76 324 74 321 C 72 312 72 300 73 290 Z",
    },
    MusclePath {
        muscle: "gastrocnemius",
        d: "M127 290 C 123 286 118 284 114 285 C 113 298 114 312 117 324 C 120 325 124 324 126 321 C 128 312 128 300 127 290 Z",
    },
];

pub(super) const BACK_PATHS: &[MusclePath] = &[
    MusclePath {
        muscle: "upper-traps",
        d: "M100 48 C 94 52 88 56 78 60 C 84 62 90 64 94 66 C 98 68 102 68 106 66 C 110 64 116 62 122 60 C 112 56 106 52 100 48 Z",
    },
    MusclePath {
        muscle: "mid-traps",
        d: "M92 64 C 97 67 103 67 108 64 C 106 72 105 78 104 84 C 101 86 99 86 96 84 C 95 78 94 72 92 64 Z",
    },
    MusclePath {
        muscle: "lower-traps",
        d: "M96 86 C 99 88 101 88 104 86 C 103 91 102 96 100 100 C 98 96 97 91 96 86 Z",
    },
    MusclePath {
        muscle: "posterior-delts",
        d: "M60 66 C 66 61 73 60 78 62 C 74 70 71 80 70 89 C 65 87 60 83 57 78 C 57 73 58 69 60 66 Z",
    },
    MusclePath {
        muscle: "posterior-delts",
        d: "M140 66 C 134 61 127 60 122 62 C 126 70 129 80 130 89 C 135 87 140 83 143 78 C 143 73 142 69 140 66 Z",
    },
    MusclePath {
        muscle: "lats",
        d: "M78 64 C 83 70 88 80 92 90 C 95 104 97 122 97 138 C 90 134 82 126 77 116 C 73 106 72 88 74 74 C 75 70 76 66 78 64 Z",
    },
    MusclePath {
        muscle: "lats",
        d: "M122 64 C 117 70 112 80 108 90 C 105 104 103 122 103 138 C 110 134 118 126 123 116 C 127 106 128 88 126 74 C 125 70 124 66 122 64 Z",
    },
    MusclePath {
        muscle: "rhomboids",
        d: "M88 68 C 91 70 94 72 96 74 C 96 80 95 86 94 90 C 91 88 88 85 86 82 C 86 77 87 72 88 68 Z",
    },
    MusclePath {
        muscle: "rhomboids",
        d: "M112 68 C 109 70 106 72 104 74 C 104 80 105 86 106 90 C 109 88 112 85 114 82 C 114 77 113 72 112 68 Z",
    },
    MusclePath {
        muscle: "spinal-erectors",
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
        muscle: "forearm-extensors",
        d: "M54 130 C 57 134 61 136 64 137 C 62 150 59 162 56 172 C 53 174 50 175 47 175 C 46 165 48 148 54 130 Z",
    },
    MusclePath {
        muscle: "forearm-extensors",
        d: "M146 130 C 143 134 139 136 136 137 C 138 150 141 162 144 172 C 147 174 150 175 153 175 C 154 165 152 148 146 130 Z",
    },
    MusclePath {
        muscle: "glute-med",
        d: "M80 160 C 85 158 92 158 97 159 C 97 162 97 165 96 167 C 90 165 84 165 79 167 C 79 164 79 162 80 160 Z",
    },
    MusclePath {
        muscle: "glute-med",
        d: "M120 160 C 115 158 108 158 103 159 C 103 162 103 165 104 167 C 110 165 116 165 121 167 C 121 164 121 162 120 160 Z",
    },
    MusclePath {
        muscle: "glute-max",
        d: "M79 168 C 84 166 90 166 96 168 C 98 175 97 182 96 188 C 90 193 83 193 78 188 C 75 181 76 173 79 168 Z",
    },
    MusclePath {
        muscle: "glute-max",
        d: "M121 168 C 116 166 110 166 104 168 C 102 175 103 182 104 188 C 110 193 117 193 122 188 C 125 181 124 173 121 168 Z",
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
        muscle: "gastrocnemius",
        d: "M72 288 C 77 282 84 280 89 282 C 90 290 90 300 89 308 C 84 310 77 310 73 308 C 71 301 71 294 72 288 Z",
    },
    MusclePath {
        muscle: "gastrocnemius",
        d: "M128 288 C 123 282 116 280 111 282 C 110 290 110 300 111 308 C 116 310 123 310 127 308 C 129 301 129 294 128 288 Z",
    },
    MusclePath {
        muscle: "soleus",
        d: "M73 310 C 78 312 84 312 89 310 C 88 316 87 322 86 326 C 82 328 77 327 74 323 C 73 319 73 314 73 310 Z",
    },
    MusclePath {
        muscle: "soleus",
        d: "M127 310 C 122 312 116 312 111 310 C 112 316 113 322 114 326 C 118 328 123 327 126 323 C 127 319 127 314 127 310 Z",
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
                    "These exercises have no muscle weights yet, so there is no map to draw."
                </p>
            } else {
                // At the workout page's gutter breakpoint the map lives in
                // a 14.5rem side panel, so the figures shrink to sit two-up
                // with the lists wrapping beneath them.
                <div class="flex flex-wrap items-start gap-x-8 gap-y-5 min-[90rem]:gap-x-4">
                    muscle_figure(
                        paths: FRONT_PATHS,
                        caption: "front",
                        involvement: involvement,
                        compact: false
                    )
                    muscle_figure(
                        paths: BACK_PATHS,
                        caption: "back",
                        involvement: involvement,
                        compact: false
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

/// Compact front/back figures for heatmap day preview popovers. One-line
/// primary/secondary labels replace the full legend; empty involvement
/// renders nothing so the card can skip the muscles block entirely.
#[component]
pub(super) async fn muscle_map_compact(involvement: &MuscleInvolvement) -> Result {
    if involvement.is_empty() {
        return view! {};
    }
    let primary_list = label_list(&involvement.primary);
    let secondary_list = label_list(&involvement.secondary);
    view! {
        <div class="mt-[0.55rem]" aria-hidden="true">
            <div class="flex items-start gap-x-3">
                muscle_figure(
                    paths: FRONT_PATHS,
                    caption: "front",
                    involvement: involvement,
                    compact: true
                )
                muscle_figure(
                    paths: BACK_PATHS,
                    caption: "back",
                    involvement: involvement,
                    compact: true
                )
            </div>
            if !involvement.primary.is_empty() {
                <p class="mt-[0.4rem] font-meta text-[0.62rem] leading-[1.4] text-ink2">
                    <span class="text-muted">"primary · "</span>
                    (primary_list.as_str())
                </p>
            }
            if !involvement.secondary.is_empty() {
                <p class="mt-[0.15rem] font-meta text-[0.62rem] leading-[1.4] text-ink2">
                    <span class="text-muted">"secondary · "</span>
                    (secondary_list.as_str())
                </p>
            }
        </div>
    }
}

#[component]
pub(super) async fn muscle_figure(
    paths: &'static [MusclePath],
    caption: &str,
    involvement: &MuscleInvolvement,
    #[default(false)] compact: bool,
) -> Result {
    let figure_class = if compact {
        "m-0 w-[4.4rem] flex-none"
    } else {
        "m-0 w-[8.5rem] flex-none sm:w-[9.5rem] min-[90rem]:w-[6.6rem]"
    };
    let caption_class = if compact {
        "mt-1 text-center font-meta text-[0.55rem] leading-none tracking-[0.13em] uppercase text-muted"
    } else {
        FIGURE_CAPTION
    };
    view! {
        <figure class=(figure_class) aria-hidden="true">
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
            <figcaption class=(caption_class)>(caption)</figcaption>
        </figure>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights(entries: &[(&str, &[(&'static str, u32)])]) -> ExerciseWeights {
        entries
            .iter()
            .map(|(name, pairs)| ((*name).to_string(), pairs.to_vec()))
            .collect()
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
                    failure: false,
                    distance_milli: None,
                    set_time_seconds: None,
                    set_type: "NORMAL_SET".into(),
                    records: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn threshold_splits_primary_from_secondary_at_seventy_five() {
        let map = weights(&[(
            "Bench Press",
            &[("mid-chest", 100), ("upper-chest", 75), ("triceps", 74)],
        )]);
        let involvement = involvement_for_exercises(["Bench Press"], &map);
        assert_eq!(involvement.primary, vec!["upper-chest", "mid-chest"]);
        assert_eq!(involvement.secondary, vec!["triceps"]);
    }

    #[test]
    fn workout_union_promotes_primary_over_secondary() {
        let map = weights(&[
            (
                "Overhead Press (Barbell)",
                &[("anterior-delts", 100), ("triceps", 50)],
            ),
            ("Skull Crusher", &[("triceps", 100)]),
            // "Leg Press" stays unweighted, mirroring a real seeding gap.
        ]);
        let workout = workout_with(&["Overhead Press (Barbell)", "Skull Crusher", "Leg Press"]);
        let involvement = workout_involvement(&workout, &map);
        assert_eq!(
            involvement.primary,
            vec!["anterior-delts", "triceps"],
            "triceps is secondary for the press but primary for the extension"
        );
        assert!(involvement.secondary.is_empty());
    }

    #[test]
    fn output_follows_the_taxonomy_order_and_empty_reports_empty() {
        let map = weights(&[
            (
                "Deadlift (Barbell)",
                &[("hamstrings", 100), ("glute-max", 100), ("upper-traps", 40)],
            ),
            (
                "Bench Press (Barbell)",
                &[("triceps", 55), ("mid-chest", 100)],
            ),
        ]);
        let workout = workout_with(&["Deadlift (Barbell)", "Bench Press (Barbell)"]);
        let involvement = workout_involvement(&workout, &map);
        assert_eq!(
            involvement.primary,
            vec!["mid-chest", "hamstrings", "glute-max"],
            "taxonomy display order, not alphabetical"
        );
        assert_eq!(involvement.secondary, vec!["upper-traps", "triceps"]);
        assert!(!involvement.is_empty());

        let unweighted =
            workout_involvement(&workout_with(&["Leg Press"]), &ExerciseWeights::new());
        assert!(unweighted.is_empty());
    }

    #[test]
    fn every_diagram_path_uses_canonical_ids_and_covers_the_vocabulary() {
        for path in FRONT_PATHS.iter().chain(BACK_PATHS) {
            assert!(
                muscle_taxonomy::canonical_muscle(path.muscle).is_some(),
                "unknown muscle {} in diagram",
                path.muscle
            );
        }
        // Every granular muscle appears somewhere on the two figures.
        for (muscle, _) in muscle_taxonomy::muscles() {
            assert!(
                FRONT_PATHS
                    .iter()
                    .chain(BACK_PATHS)
                    .any(|path| path.muscle == muscle),
                "muscle {muscle} missing from the diagram"
            );
        }
    }
}
