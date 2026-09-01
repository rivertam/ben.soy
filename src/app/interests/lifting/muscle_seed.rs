//! Default exercise↔muscle weights.
//!
//! [`SEED_WEIGHTS`] holds researched ratios per exercise name; exercises the
//! table doesn't know fall back to [`derived_weights`], which reconstructs
//! the pre-weight primary/secondary split from the exercise's taxonomy tags
//! (primaries → 100, the rest → 50) and expands each coarse tag to its
//! granular constituents. Seeding is insert-only — the reconcile in
//! `archive/db.rs` never touches an exercise that already has weight rows,
//! so admin-tuned ratios are authoritative (`docs/fitness.md`).

use std::collections::BTreeMap;

use super::muscle_taxonomy;

/// The muscles a movement pattern trains as prime movers, out of the coarse
/// muscles the taxonomy tags alongside it. This is the old
/// `muscles::PRIMARY_BY_MOVEMENT` table, retired from rendering and kept
/// only to derive fallback weights for exercises the seed table doesn't know.
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

/// Researched ratios: exercise name → [(granular muscle id, ratio 1..=100)].
/// Authored from the archive's full exercise list; rubric: prime movers
/// 75–100, strong synergists 40–70, stabilizers 10–35, below 10 omitted.
/// A pure-cardio exercise is deliberately absent (no muscle credit).
const SEED_WEIGHTS: &[(&str, &[(&str, u8)])] = &[
    (
        "Arnold Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 60),
            ("upper-traps", 20),
            ("lower-traps", 18),
            ("upper-chest", 20),
            ("serratus-anterior", 15),
            ("triceps", 50),
        ],
    ),
    (
        "Assisted Chest Dip",
        &[
            ("anterior-delts", 50),
            ("lats", 10),
            ("upper-chest", 15),
            ("mid-chest", 60),
            ("lower-chest", 100),
            ("serratus-anterior", 15),
            ("triceps", 65),
        ],
    ),
    (
        "Atlantis Overhead Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 60),
            ("upper-traps", 15),
            ("lower-traps", 20),
            ("upper-chest", 25),
            ("serratus-anterior", 20),
            ("triceps", 55),
        ],
    ),
    (
        "Barbell Biceps Curl",
        &[
            ("anterior-delts", 10),
            ("biceps", 100),
            ("brachialis", 45),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Barbell Jefferson Curl",
        &[
            ("spinal-erectors", 100),
            ("forearm-flexors", 20),
            ("abs", 10),
            ("hamstrings", 55),
            ("glute-max", 35),
        ],
    ),
    (
        "Barbell KAS Glute Bridge (female)",
        &[
            ("spinal-erectors", 20),
            ("abs", 10),
            ("quads", 15),
            ("hamstrings", 40),
            ("adductors", 15),
            ("glute-max", 100),
        ],
    ),
    (
        "Barbell Lying Triceps Extension Skull Crusher",
        &[("triceps", 100), ("forearm-flexors", 10)],
    ),
    (
        "Barbell Resurrection Lifts",
        &[
            ("lats", 60),
            ("mid-chest", 20),
            ("serratus-anterior", 25),
            ("triceps", 25),
            ("abs", 100),
            ("obliques", 20),
            ("hip-flexors", 10),
        ],
    ),
    (
        "Barbell Seated Overhead Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 55),
            ("upper-traps", 30),
            ("lower-traps", 25),
            ("upper-chest", 20),
            ("serratus-anterior", 20),
            ("triceps", 60),
            ("abs", 10),
        ],
    ),
    (
        "Barbell Standing Military Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 55),
            ("upper-traps", 30),
            ("lower-traps", 25),
            ("spinal-erectors", 15),
            ("upper-chest", 25),
            ("serratus-anterior", 20),
            ("triceps", 55),
            ("abs", 25),
            ("obliques", 15),
        ],
    ),
    (
        "Barbell Zercher Deadlift",
        &[
            ("spinal-erectors", 90),
            ("biceps", 30),
            ("brachialis", 15),
            ("abs", 40),
            ("obliques", 15),
            ("quads", 75),
            ("hamstrings", 65),
            ("adductors", 25),
            ("glute-max", 100),
        ],
    ),
    (
        "Barbell Zercher Squat",
        &[
            ("upper-traps", 15),
            ("spinal-erectors", 55),
            ("biceps", 15),
            ("abs", 40),
            ("obliques", 25),
            ("quads", 100),
            ("hamstrings", 30),
            ("adductors", 45),
            ("glute-max", 80),
            ("glute-med", 15),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Bench Press",
        &[
            ("anterior-delts", 50),
            ("upper-chest", 50),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Bench Press (Cambered Bar)",
        &[
            ("anterior-delts", 50),
            ("upper-chest", 50),
            ("mid-chest", 100),
            ("lower-chest", 45),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Bent Over One Arm Row (Dumbbell)",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 60),
            ("lats", 100),
            ("rhomboids", 55),
            ("spinal-erectors", 20),
            ("biceps", 45),
            ("brachialis", 35),
            ("forearm-flexors", 35),
            ("obliques", 15),
        ],
    ),
    (
        "Bent Over Row",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 60),
            ("lower-traps", 25),
            ("lats", 100),
            ("rhomboids", 60),
            ("spinal-erectors", 50),
            ("biceps", 50),
            ("brachialis", 35),
            ("forearm-flexors", 35),
            ("hamstrings", 15),
        ],
    ),
    (
        "Bicep Twist Curl (Cable)",
        &[("biceps", 100), ("brachialis", 40), ("forearm-flexors", 20)],
    ),
    (
        "Bicycle Crunch (male)",
        &[("abs", 100), ("obliques", 80), ("hip-flexors", 40)],
    ),
    (
        "Booty Builder",
        &[
            ("spinal-erectors", 10),
            ("quads", 15),
            ("hamstrings", 40),
            ("adductors", 15),
            ("glute-max", 100),
            ("glute-med", 30),
        ],
    ),
    (
        "Bulgarian Split Squat",
        &[
            ("spinal-erectors", 15),
            ("abs", 10),
            ("quads", 100),
            ("hamstrings", 30),
            ("adductors", 35),
            ("glute-max", 90),
            ("glute-med", 45),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Bulgarian Split Squat (Smith Machine)",
        &[
            ("quads", 100),
            ("hamstrings", 25),
            ("adductors", 30),
            ("glute-max", 85),
            ("glute-med", 40),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Burpee",
        &[
            ("anterior-delts", 40),
            ("mid-chest", 50),
            ("triceps", 45),
            ("abs", 40),
            ("hip-flexors", 35),
            ("quads", 100),
            ("hamstrings", 25),
            ("glute-max", 60),
            ("gastrocnemius", 30),
            ("soleus", 15),
        ],
    ),
    (
        "Cable Crossover",
        &[
            ("anterior-delts", 35),
            ("upper-chest", 35),
            ("mid-chest", 100),
            ("lower-chest", 50),
            ("serratus-anterior", 15),
            ("biceps", 10),
        ],
    ),
    (
        "Cable Crossover (Upward)",
        &[
            ("anterior-delts", 50),
            ("upper-chest", 100),
            ("mid-chest", 45),
            ("serratus-anterior", 15),
            ("biceps", 15),
        ],
    ),
    (
        "Cable Curl",
        &[("biceps", 100), ("brachialis", 40), ("forearm-flexors", 25)],
    ),
    ("Cable Kneeling Crunch", &[("abs", 100), ("obliques", 25)]),
    (
        "Cable One Arm Biceps Curl",
        &[
            ("anterior-delts", 10),
            ("biceps", 100),
            ("brachialis", 40),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Cable One Arm Lateral Raise",
        &[
            ("anterior-delts", 25),
            ("lateral-delts", 100),
            ("posterior-delts", 20),
            ("upper-traps", 25),
            ("serratus-anterior", 10),
        ],
    ),
    (
        "Cable One Arm Reverse Fly",
        &[
            ("lateral-delts", 15),
            ("posterior-delts", 100),
            ("mid-traps", 50),
            ("rhomboids", 45),
        ],
    ),
    ("Cable One Arm Wrist Curl", &[("forearm-flexors", 100)]),
    (
        "Cable Overhead Single Arm Triceps Extension (rope attachment)",
        &[("triceps", 100), ("forearm-extensors", 15), ("abs", 10)],
    ),
    (
        "Cable Pulldown Bicep Curl",
        &[("biceps", 100), ("brachialis", 35), ("forearm-flexors", 20)],
    ),
    (
        "Cable Reverse Grip Pulldown",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 30),
            ("lower-traps", 35),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 65),
            ("brachialis", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Cable Reverse One Arm Curl",
        &[
            ("biceps", 30),
            ("brachialis", 100),
            ("forearm-extensors", 60),
        ],
    ),
    (
        "Cable Romanian Deadlift (female)",
        &[
            ("upper-traps", 15),
            ("spinal-erectors", 70),
            ("forearm-flexors", 15),
            ("hamstrings", 100),
            ("adductors", 20),
            ("glute-max", 90),
        ],
    ),
    (
        "Cable Seated Pullover",
        &[
            ("posterior-delts", 15),
            ("lats", 100),
            ("lower-chest", 20),
            ("serratus-anterior", 15),
            ("triceps", 30),
            ("abs", 15),
        ],
    ),
    (
        "Cable Standing Back Wrist Curl",
        &[("forearm-extensors", 100)],
    ),
    (
        "Cable Standing Face Pull",
        &[
            ("posterior-delts", 100),
            ("upper-traps", 20),
            ("mid-traps", 60),
            ("lower-traps", 40),
            ("rhomboids", 55),
            ("biceps", 15),
            ("forearm-flexors", 10),
        ],
    ),
    (
        "Cable Standing Fly",
        &[
            ("anterior-delts", 40),
            ("upper-chest", 25),
            ("mid-chest", 100),
            ("lower-chest", 35),
            ("serratus-anterior", 15),
            ("biceps", 15),
        ],
    ),
    (
        "Cable Standing Up Straight Crossovers",
        &[
            ("anterior-delts", 35),
            ("upper-chest", 15),
            ("mid-chest", 100),
            ("lower-chest", 55),
            ("serratus-anterior", 15),
        ],
    ),
    (
        "Cable Twist",
        &[("spinal-erectors", 15), ("abs", 50), ("obliques", 100)],
    ),
    (
        "Cable Wide-Grip Lat Pulldown",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 30),
            ("lower-traps", 30),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 50),
            ("brachialis", 40),
            ("forearm-flexors", 35),
        ],
    ),
    (
        "Captains Chair Straight Leg Raise",
        &[
            ("abs", 70),
            ("obliques", 20),
            ("hip-flexors", 100),
            ("quads", 15),
        ],
    ),
    (
        "Chest Dip",
        &[
            ("anterior-delts", 45),
            ("mid-chest", 60),
            ("lower-chest", 100),
            ("serratus-anterior", 15),
            ("triceps", 65),
            ("abs", 10),
        ],
    ),
    (
        "Chest Dip (Assisted)",
        &[
            ("anterior-delts", 50),
            ("mid-chest", 55),
            ("lower-chest", 100),
            ("serratus-anterior", 15),
            ("triceps", 65),
        ],
    ),
    (
        "Chest Fly (Plate Machine)",
        &[
            ("anterior-delts", 40),
            ("upper-chest", 35),
            ("mid-chest", 100),
            ("lower-chest", 35),
            ("serratus-anterior", 10),
            ("biceps", 10),
        ],
    ),
    (
        "Chest Fly - Arms",
        &[
            ("anterior-delts", 40),
            ("upper-chest", 35),
            ("mid-chest", 100),
            ("lower-chest", 35),
            ("serratus-anterior", 10),
            ("biceps", 10),
        ],
    ),
    (
        "Chest Press (Vertical Plate Machine)",
        &[
            ("anterior-delts", 50),
            ("upper-chest", 45),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Chest Supported Land Mine Shrugs",
        &[
            ("upper-traps", 100),
            ("mid-traps", 45),
            ("rhomboids", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Chest-Supported Land Mine Row",
        &[
            ("posterior-delts", 50),
            ("mid-traps", 60),
            ("lower-traps", 20),
            ("lats", 100),
            ("rhomboids", 60),
            ("biceps", 50),
            ("brachialis", 35),
            ("forearm-flexors", 30),
        ],
    ),
    (
        "Chest-supported Row",
        &[
            ("posterior-delts", 50),
            ("mid-traps", 65),
            ("lower-traps", 20),
            ("lats", 100),
            ("rhomboids", 65),
            ("biceps", 40),
            ("brachialis", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Chest-Supported Row (Plate Machine)",
        &[
            ("posterior-delts", 50),
            ("mid-traps", 65),
            ("lower-traps", 20),
            ("lats", 100),
            ("rhomboids", 65),
            ("biceps", 40),
            ("brachialis", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Chin ups",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 30),
            ("lower-traps", 25),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 70),
            ("brachialis", 35),
            ("forearm-flexors", 35),
            ("abs", 15),
        ],
    ),
    (
        "Close-Grip Front Lat Pulldown",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 30),
            ("lower-traps", 25),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 55),
            ("brachialis", 40),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Criss Cross Upper Chest Raise",
        &[
            ("anterior-delts", 45),
            ("upper-chest", 100),
            ("mid-chest", 30),
            ("serratus-anterior", 15),
            ("biceps", 10),
        ],
    ),
    (
        "Cross Body Hammer Curl",
        &[
            ("biceps", 60),
            ("brachialis", 100),
            ("forearm-flexors", 30),
            ("forearm-extensors", 20),
        ],
    ),
    (
        "Deadlift",
        &[
            ("upper-traps", 40),
            ("lats", 30),
            ("spinal-erectors", 90),
            ("forearm-flexors", 40),
            ("abs", 15),
            ("quads", 50),
            ("hamstrings", 85),
            ("glute-max", 100),
        ],
    ),
    (
        "Decline Crunch",
        &[("abs", 100), ("obliques", 25), ("hip-flexors", 15)],
    ),
    (
        "Decline Shrug",
        &[
            ("upper-traps", 100),
            ("mid-traps", 40),
            ("rhomboids", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Deficit Pushup",
        &[
            ("anterior-delts", 50),
            ("upper-chest", 30),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 25),
            ("triceps", 60),
            ("abs", 20),
        ],
    ),
    (
        "Deficit Romanian Deadlift",
        &[
            ("upper-traps", 25),
            ("lats", 25),
            ("spinal-erectors", 80),
            ("forearm-flexors", 40),
            ("abs", 10),
            ("hamstrings", 100),
            ("adductors", 20),
            ("glute-max", 90),
        ],
    ),
    (
        "Deficit Split Squat (Smith Machine)",
        &[
            ("quads", 100),
            ("hamstrings", 30),
            ("adductors", 40),
            ("glute-max", 85),
            ("glute-med", 35),
            ("gastrocnemius", 10),
            ("soleus", 10),
        ],
    ),
    (
        "Deficit Sumo Deadlift (Barbell)",
        &[
            ("upper-traps", 40),
            ("lats", 30),
            ("spinal-erectors", 80),
            ("forearm-flexors", 40),
            ("abs", 15),
            ("quads", 80),
            ("hamstrings", 65),
            ("adductors", 60),
            ("glute-max", 100),
            ("glute-med", 25),
        ],
    ),
    (
        "Dumbbell Assisted Bulgarian Split Squat",
        &[
            ("abs", 10),
            ("quads", 100),
            ("hamstrings", 30),
            ("adductors", 25),
            ("glute-max", 85),
            ("glute-med", 40),
        ],
    ),
    (
        "Dumbbell Bench Press",
        &[
            ("anterior-delts", 55),
            ("upper-chest", 45),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 15),
            ("triceps", 50),
        ],
    ),
    (
        "Dumbbell Biceps Curl",
        &[
            ("anterior-delts", 10),
            ("biceps", 100),
            ("brachialis", 45),
            ("forearm-flexors", 20),
        ],
    ),
    (
        "Dumbbell Concentration Curl",
        &[("biceps", 100), ("brachialis", 35), ("forearm-flexors", 20)],
    ),
    (
        "Dumbbell Fly",
        &[
            ("anterior-delts", 40),
            ("upper-chest", 35),
            ("mid-chest", 100),
            ("lower-chest", 35),
            ("serratus-anterior", 10),
            ("biceps", 10),
        ],
    ),
    (
        "Dumbbell Incline Bench Press",
        &[
            ("anterior-delts", 65),
            ("upper-chest", 100),
            ("mid-chest", 55),
            ("serratus-anterior", 15),
            ("triceps", 50),
        ],
    ),
    (
        "Dumbbell Incline Curl",
        &[
            ("anterior-delts", 10),
            ("biceps", 100),
            ("brachialis", 30),
            ("forearm-flexors", 20),
        ],
    ),
    (
        "Dumbbell Preacher Curl",
        &[("biceps", 100), ("brachialis", 45), ("forearm-flexors", 20)],
    ),
    (
        "Dumbbell Pullover (VERSION 2)",
        &[
            ("posterior-delts", 15),
            ("lats", 100),
            ("mid-chest", 55),
            ("lower-chest", 40),
            ("serratus-anterior", 35),
            ("triceps", 40),
            ("abs", 15),
        ],
    ),
    (
        "Dumbbell Seated Palms Up Wrist Curl",
        &[("forearm-flexors", 100)],
    ),
    (
        "Dumbbell Split Stance Single Arm Overhead Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 55),
            ("upper-traps", 25),
            ("lower-traps", 20),
            ("serratus-anterior", 20),
            ("triceps", 50),
            ("abs", 20),
            ("obliques", 30),
        ],
    ),
    (
        "Dumbbell Standing Overhead Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 60),
            ("upper-traps", 30),
            ("lower-traps", 25),
            ("spinal-erectors", 15),
            ("upper-chest", 25),
            ("serratus-anterior", 25),
            ("triceps", 55),
            ("abs", 20),
            ("obliques", 15),
        ],
    ),
    (
        "Dumbbell Walking Lunges",
        &[
            ("upper-traps", 12),
            ("spinal-erectors", 15),
            ("forearm-flexors", 25),
            ("abs", 15),
            ("quads", 100),
            ("hamstrings", 40),
            ("adductors", 30),
            ("glute-max", 85),
            ("glute-med", 50),
            ("gastrocnemius", 15),
        ],
    ),
    (
        "EZ Bar Lying Bent Arms Pullover",
        &[
            ("posterior-delts", 15),
            ("lats", 100),
            ("mid-chest", 45),
            ("lower-chest", 35),
            ("serratus-anterior", 35),
            ("triceps", 45),
            ("abs", 10),
        ],
    ),
    (
        "EZ Barbell Reverse grip Preacher Curl",
        &[
            ("biceps", 45),
            ("brachialis", 100),
            ("forearm-flexors", 15),
            ("forearm-extensors", 50),
        ],
    ),
    (
        "EZ Barbell Standing Wrist Reverse Curl",
        &[("forearm-extensors", 100)],
    ),
    (
        "EZ-bar Drag Bicep Curl",
        &[
            ("posterior-delts", 10),
            ("biceps", 100),
            ("brachialis", 45),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Face Pull (Cable)",
        &[
            ("posterior-delts", 100),
            ("upper-traps", 20),
            ("mid-traps", 60),
            ("lower-traps", 40),
            ("rhomboids", 55),
            ("biceps", 20),
            ("forearm-flexors", 15),
        ],
    ),
    (
        "Farmer's Walk (Dumbbells)",
        &[
            ("upper-traps", 75),
            ("spinal-erectors", 40),
            ("forearm-flexors", 100),
            ("abs", 35),
            ("obliques", 35),
            ("quads", 20),
            ("hamstrings", 15),
            ("glute-max", 20),
            ("glute-med", 30),
            ("gastrocnemius", 20),
            ("soleus", 20),
        ],
    ),
    (
        "Flexion Row",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 60),
            ("lats", 100),
            ("rhomboids", 60),
            ("biceps", 40),
            ("brachialis", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Front Plank",
        &[
            ("anterior-delts", 10),
            ("spinal-erectors", 15),
            ("serratus-anterior", 15),
            ("abs", 100),
            ("obliques", 45),
            ("hip-flexors", 20),
            ("quads", 10),
        ],
    ),
    (
        "Front Raise",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 30),
            ("upper-traps", 15),
            ("upper-chest", 20),
            ("serratus-anterior", 15),
        ],
    ),
    (
        "Full Squat",
        &[
            ("spinal-erectors", 45),
            ("abs", 15),
            ("quads", 100),
            ("hamstrings", 25),
            ("adductors", 50),
            ("glute-max", 80),
            ("glute-med", 20),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Glute Hip Thrust (Atlantis)",
        &[
            ("spinal-erectors", 15),
            ("quads", 20),
            ("hamstrings", 45),
            ("adductors", 10),
            ("glute-max", 100),
            ("glute-med", 15),
        ],
    ),
    (
        "Glute Kickback (Machine)",
        &[
            ("spinal-erectors", 15),
            ("hamstrings", 40),
            ("glute-max", 100),
            ("glute-med", 20),
        ],
    ),
    (
        "Good Morning (Squat Machine)",
        &[
            ("spinal-erectors", 90),
            ("hamstrings", 100),
            ("adductors", 15),
            ("glute-max", 80),
        ],
    ),
    (
        "Grip Roller",
        &[
            ("anterior-delts", 15),
            ("forearm-flexors", 100),
            ("forearm-extensors", 60),
        ],
    ),
    (
        "Hammer Curl",
        &[("biceps", 70), ("brachialis", 100), ("forearm-flexors", 30)],
    ),
    (
        "Hanging Leg Raise",
        &[
            ("lats", 10),
            ("forearm-flexors", 25),
            ("abs", 100),
            ("obliques", 35),
            ("hip-flexors", 85),
        ],
    ),
    (
        "Hanging Toes to Bar",
        &[
            ("lats", 20),
            ("serratus-anterior", 10),
            ("forearm-flexors", 30),
            ("abs", 100),
            ("obliques", 40),
            ("hip-flexors", 75),
        ],
    ),
    (
        "Hip Abductor (Machine)",
        &[("glute-max", 30), ("glute-med", 100)],
    ),
    ("Hip Adductor (Machine)", &[("adductors", 100)]),
    (
        "Hip Thrust",
        &[
            ("spinal-erectors", 15),
            ("quads", 20),
            ("hamstrings", 40),
            ("adductors", 15),
            ("glute-max", 100),
            ("glute-med", 30),
        ],
    ),
    (
        "Incline Bench Press",
        &[
            ("anterior-delts", 65),
            ("upper-chest", 100),
            ("mid-chest", 50),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Incline Bench Press (Cambered Bar)",
        &[
            ("anterior-delts", 65),
            ("upper-chest", 100),
            ("mid-chest", 50),
            ("serratus-anterior", 15),
            ("triceps", 50),
        ],
    ),
    (
        "Incline Chest Fly (Dumbbell)",
        &[
            ("anterior-delts", 45),
            ("upper-chest", 100),
            ("mid-chest", 45),
            ("serratus-anterior", 10),
            ("biceps", 15),
        ],
    ),
    (
        "Incline Push Up Depth Jump",
        &[
            ("anterior-delts", 50),
            ("mid-chest", 100),
            ("lower-chest", 60),
            ("serratus-anterior", 30),
            ("triceps", 65),
            ("abs", 20),
        ],
    ),
    (
        "Incline Row (Atlantis)",
        &[
            ("posterior-delts", 55),
            ("mid-traps", 75),
            ("lats", 100),
            ("rhomboids", 70),
            ("biceps", 50),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    ("Inner Thigh Machine (Roc-It)", &[("adductors", 100)]),
    (
        "Inverted Row (Bodyweight)",
        &[
            ("posterior-delts", 50),
            ("mid-traps", 70),
            ("lats", 100),
            ("rhomboids", 65),
            ("spinal-erectors", 20),
            ("biceps", 55),
            ("brachialis", 30),
            ("forearm-flexors", 30),
            ("abs", 15),
            ("glute-max", 15),
        ],
    ),
    (
        "Iso-Lateral Chest Press (Machine)",
        &[
            ("anterior-delts", 50),
            ("upper-chest", 45),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Iso-Lateral High Row",
        &[
            ("posterior-delts", 40),
            ("mid-traps", 45),
            ("lower-traps", 20),
            ("lats", 100),
            ("rhomboids", 45),
            ("biceps", 45),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Landmine Kelso Shrug",
        &[
            ("posterior-delts", 20),
            ("upper-traps", 30),
            ("mid-traps", 100),
            ("lower-traps", 45),
            ("rhomboids", 80),
            ("spinal-erectors", 25),
            ("forearm-flexors", 15),
        ],
    ),
    (
        "Lat Pull-around",
        &[
            ("posterior-delts", 25),
            ("lower-traps", 20),
            ("lats", 100),
            ("biceps", 15),
            ("forearm-flexors", 15),
        ],
    ),
    (
        "Lat Pulldown (Single Arm)",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 30),
            ("lower-traps", 25),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 50),
            ("brachialis", 35),
            ("forearm-flexors", 25),
            ("obliques", 15),
        ],
    ),
    (
        "Lat Pulldown - Underhand (Cable)",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 30),
            ("lower-traps", 35),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 65),
            ("brachialis", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Lateral Raise",
        &[
            ("anterior-delts", 20),
            ("lateral-delts", 100),
            ("posterior-delts", 15),
            ("upper-traps", 20),
            ("serratus-anterior", 10),
        ],
    ),
    (
        "Lever Back Extension",
        &[
            ("spinal-erectors", 100),
            ("hamstrings", 50),
            ("glute-max", 60),
        ],
    ),
    (
        "Lever Bicep Curl",
        &[("biceps", 100), ("brachialis", 40), ("forearm-flexors", 20)],
    ),
    (
        "Lever Biceps Curl",
        &[("biceps", 100), ("brachialis", 45), ("forearm-flexors", 20)],
    ),
    (
        "Lever Chest Press",
        &[
            ("anterior-delts", 50),
            ("upper-chest", 45),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Lever Front Pulldown",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 30),
            ("lower-traps", 35),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 55),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Lever Glute Abductors Press",
        &[("glute-max", 40), ("glute-med", 100)],
    ),
    (
        "Lever Horizontal One leg Press",
        &[
            ("quads", 100),
            ("hamstrings", 25),
            ("adductors", 35),
            ("glute-max", 75),
            ("glute-med", 20),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Lever Incline Chest Press",
        &[
            ("anterior-delts", 60),
            ("upper-chest", 100),
            ("mid-chest", 50),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Lever Kneeling Leg Curl",
        &[
            ("hamstrings", 100),
            ("glute-max", 10),
            ("gastrocnemius", 25),
        ],
    ),
    (
        "Lever Lateral Raise",
        &[
            ("anterior-delts", 20),
            ("lateral-delts", 100),
            ("posterior-delts", 15),
            ("upper-traps", 20),
        ],
    ),
    ("Lever Leg Extension", &[("quads", 100)]),
    (
        "Lever Low Row",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 55),
            ("lats", 100),
            ("rhomboids", 60),
            ("biceps", 45),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Lever Lying Leg Curl",
        &[("hamstrings", 100), ("gastrocnemius", 25)],
    ),
    (
        "Lever Lying Single Leg Curl",
        &[("hamstrings", 100), ("gastrocnemius", 30)],
    ),
    (
        "Lever Narrow Grip Seated Row",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 60),
            ("lower-traps", 15),
            ("lats", 100),
            ("rhomboids", 60),
            ("spinal-erectors", 15),
            ("biceps", 45),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    ("Lever One Leg Extension", &[("quads", 100)]),
    (
        "Lever Pec Deck Fly",
        &[
            ("anterior-delts", 35),
            ("upper-chest", 40),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 15),
        ],
    ),
    (
        "Lever Preacher Curl",
        &[("biceps", 100), ("brachialis", 45), ("forearm-flexors", 20)],
    ),
    (
        "Lever Pronated Grip Seated Scapular Retraction Shrug",
        &[
            ("posterior-delts", 25),
            ("upper-traps", 45),
            ("mid-traps", 100),
            ("rhomboids", 90),
            ("forearm-flexors", 15),
        ],
    ),
    (
        "Lever Pronated Grip Seated Scapular Retraction Shrug (plate loaded)",
        &[
            ("posterior-delts", 25),
            ("upper-traps", 45),
            ("mid-traps", 100),
            ("rhomboids", 90),
            ("forearm-flexors", 15),
        ],
    ),
    (
        "Lever Pullover",
        &[
            ("posterior-delts", 15),
            ("lats", 100),
            ("mid-chest", 20),
            ("lower-chest", 30),
            ("serratus-anterior", 30),
            ("triceps", 30),
            ("abs", 10),
        ],
    ),
    (
        "Lever Row",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 55),
            ("lats", 100),
            ("rhomboids", 60),
            ("biceps", 45),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Lever Seated Dip",
        &[
            ("anterior-delts", 45),
            ("mid-chest", 35),
            ("lower-chest", 55),
            ("triceps", 100),
        ],
    ),
    ("Lever Seated Hip Adduction", &[("adductors", 100)]),
    (
        "Lever Seated Leg Curl",
        &[("hamstrings", 100), ("gastrocnemius", 25)],
    ),
    (
        "Lever Seated Leg Press",
        &[
            ("quads", 100),
            ("hamstrings", 25),
            ("adductors", 30),
            ("glute-max", 65),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Lever Seated One Leg Curl",
        &[("hamstrings", 100), ("gastrocnemius", 20)],
    ),
    (
        "Lever Seated Reverse Fly",
        &[
            ("lateral-delts", 15),
            ("posterior-delts", 100),
            ("mid-traps", 55),
            ("rhomboids", 55),
        ],
    ),
    (
        "Lever Seated Row",
        &[
            ("posterior-delts", 50),
            ("mid-traps", 65),
            ("lats", 100),
            ("rhomboids", 60),
            ("biceps", 45),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Lever Seated Shoulder Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 60),
            ("upper-traps", 25),
            ("lower-traps", 20),
            ("upper-chest", 20),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Lever Shrug",
        &[
            ("upper-traps", 100),
            ("mid-traps", 30),
            ("rhomboids", 15),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Lever Standing Calf Raise",
        &[("gastrocnemius", 100), ("soleus", 45)],
    ),
    (
        "Lever Total Abdominal Crunch",
        &[("abs", 100), ("obliques", 30), ("hip-flexors", 10)],
    ),
    (
        "Lever Total Abdominal Oblique Crunch",
        &[("abs", 75), ("obliques", 100)],
    ),
    ("Lever Triceps Extension", &[("triceps", 100)]),
    (
        "Low Pull (Machine)",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 60),
            ("lats", 100),
            ("rhomboids", 60),
            ("spinal-erectors", 15),
            ("biceps", 50),
            ("brachialis", 30),
            ("forearm-flexors", 30),
        ],
    ),
    (
        "Lunge",
        &[
            ("spinal-erectors", 12),
            ("abs", 15),
            ("quads", 100),
            ("hamstrings", 35),
            ("adductors", 30),
            ("glute-max", 85),
            ("glute-med", 45),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Lying Leg Raise",
        &[("abs", 70), ("obliques", 20), ("hip-flexors", 100)],
    ),
    (
        "Medicine Ball Standing Overhead Throw",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 25),
            ("lower-traps", 15),
            ("upper-chest", 35),
            ("serratus-anterior", 25),
            ("triceps", 60),
            ("abs", 40),
            ("obliques", 25),
            ("quads", 15),
            ("glute-max", 15),
        ],
    ),
    (
        "MTS Biceps Curl",
        &[("biceps", 100), ("brachialis", 45), ("forearm-flexors", 20)],
    ),
    (
        "MTS High Row",
        &[
            ("posterior-delts", 40),
            ("mid-traps", 45),
            ("lats", 100),
            ("rhomboids", 50),
            ("biceps", 45),
            ("brachialis", 25),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "One Arm Bent-over Row",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 50),
            ("lats", 100),
            ("rhomboids", 55),
            ("spinal-erectors", 20),
            ("biceps", 50),
            ("brachialis", 35),
            ("forearm-flexors", 35),
            ("obliques", 15),
        ],
    ),
    (
        "One Arm Mid-Trap Shrug",
        &[
            ("posterior-delts", 15),
            ("upper-traps", 45),
            ("mid-traps", 100),
            ("lower-traps", 30),
            ("rhomboids", 60),
            ("forearm-flexors", 20),
        ],
    ),
    (
        "One-Armed Cable Pullover",
        &[
            ("posterior-delts", 15),
            ("lats", 100),
            ("lower-chest", 20),
            ("serratus-anterior", 25),
            ("triceps", 25),
            ("abs", 15),
            ("obliques", 10),
        ],
    ),
    (
        "Overhead Press (Machine)",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 55),
            ("upper-traps", 20),
            ("lower-traps", 20),
            ("upper-chest", 25),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Overhead Triceps Extension",
        &[
            ("anterior-delts", 10),
            ("triceps", 100),
            ("forearm-flexors", 10),
        ],
    ),
    (
        "Pec Fly (Roc It Machine)",
        &[
            ("anterior-delts", 35),
            ("upper-chest", 45),
            ("mid-chest", 100),
            ("lower-chest", 40),
            ("serratus-anterior", 15),
        ],
    ),
    (
        "Pike Push up",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 30),
            ("upper-traps", 20),
            ("upper-chest", 40),
            ("serratus-anterior", 25),
            ("triceps", 60),
            ("abs", 15),
        ],
    ),
    (
        "Power Clean",
        &[
            ("anterior-delts", 25),
            ("lateral-delts", 20),
            ("upper-traps", 70),
            ("spinal-erectors", 85),
            ("biceps", 15),
            ("forearm-flexors", 40),
            ("abs", 20),
            ("quads", 80),
            ("hamstrings", 70),
            ("glute-max", 100),
            ("gastrocnemius", 35),
            ("soleus", 25),
        ],
    ),
    (
        "Preacher Curl",
        &[("biceps", 100), ("brachialis", 45), ("forearm-flexors", 20)],
    ),
    (
        "Preacher Hammer Curl",
        &[
            ("biceps", 75),
            ("brachialis", 100),
            ("forearm-flexors", 30),
            ("forearm-extensors", 20),
        ],
    ),
    ("Preacher Reverse Wrist Curl", &[("forearm-extensors", 100)]),
    (
        "Pull up",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 35),
            ("lower-traps", 35),
            ("lats", 100),
            ("rhomboids", 35),
            ("biceps", 50),
            ("brachialis", 50),
            ("forearm-flexors", 40),
            ("abs", 15),
        ],
    ),
    (
        "Pull Up (Assisted)",
        &[
            ("posterior-delts", 20),
            ("mid-traps", 25),
            ("lower-traps", 25),
            ("lats", 100),
            ("rhomboids", 25),
            ("biceps", 55),
            ("brachialis", 35),
            ("forearm-flexors", 30),
            ("abs", 10),
        ],
    ),
    (
        "Pull Up (Parallel Grip)",
        &[
            ("posterior-delts", 30),
            ("mid-traps", 30),
            ("lower-traps", 30),
            ("lats", 100),
            ("rhomboids", 35),
            ("biceps", 60),
            ("brachialis", 55),
            ("forearm-flexors", 40),
            ("abs", 15),
        ],
    ),
    (
        "Pulldown",
        &[
            ("posterior-delts", 25),
            ("mid-traps", 25),
            ("lower-traps", 30),
            ("lats", 100),
            ("rhomboids", 30),
            ("biceps", 50),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Pullover",
        &[
            ("posterior-delts", 15),
            ("lats", 100),
            ("mid-chest", 45),
            ("serratus-anterior", 35),
            ("triceps", 30),
            ("abs", 15),
        ],
    ),
    (
        "Rear Delt Fly",
        &[
            ("lateral-delts", 15),
            ("posterior-delts", 100),
            ("mid-traps", 45),
            ("rhomboids", 45),
        ],
    ),
    (
        "Reverse Concentration Curls",
        &[
            ("biceps", 30),
            ("brachialis", 100),
            ("forearm-flexors", 15),
            ("forearm-extensors", 55),
        ],
    ),
    (
        "Reverse Curl",
        &[
            ("biceps", 30),
            ("brachialis", 100),
            ("forearm-flexors", 15),
            ("forearm-extensors", 70),
        ],
    ),
    (
        "Reverse grip machine lat pulldown",
        &[
            ("posterior-delts", 20),
            ("mid-traps", 25),
            ("lower-traps", 25),
            ("lats", 100),
            ("rhomboids", 25),
            ("biceps", 65),
            ("brachialis", 20),
            ("forearm-flexors", 25),
        ],
    ),
    ("Reverse Wrist Curl", &[("forearm-extensors", 100)]),
    (
        "Ring Neutral Grip Inverted Row",
        &[
            ("posterior-delts", 50),
            ("mid-traps", 60),
            ("lats", 100),
            ("rhomboids", 60),
            ("spinal-erectors", 15),
            ("biceps", 50),
            ("brachialis", 40),
            ("forearm-flexors", 30),
            ("abs", 20),
            ("glute-max", 10),
        ],
    ),
    (
        "Romanian Deadlift",
        &[
            ("upper-traps", 25),
            ("lats", 20),
            ("spinal-erectors", 70),
            ("forearm-flexors", 40),
            ("abs", 10),
            ("hamstrings", 100),
            ("adductors", 25),
            ("glute-max", 85),
        ],
    ),
    (
        "Romanian Deadlift (machine)",
        &[
            ("upper-traps", 20),
            ("lats", 15),
            ("spinal-erectors", 65),
            ("forearm-flexors", 25),
            ("hamstrings", 100),
            ("adductors", 20),
            ("glute-max", 80),
        ],
    ),
    (
        "Romanian Deadlift (Smith Machine)",
        &[
            ("upper-traps", 25),
            ("lats", 20),
            ("spinal-erectors", 70),
            ("forearm-flexors", 30),
            ("hamstrings", 100),
            ("adductors", 20),
            ("glute-max", 80),
        ],
    ),
    (
        "Rotary Torso Machine",
        &[("spinal-erectors", 15), ("abs", 40), ("obliques", 100)],
    ),
    (
        "Russian Twist",
        &[("abs", 60), ("obliques", 100), ("hip-flexors", 25)],
    ),
    (
        "Sandbag Power Clean",
        &[
            ("anterior-delts", 25),
            ("upper-traps", 60),
            ("lats", 15),
            ("spinal-erectors", 75),
            ("biceps", 30),
            ("forearm-flexors", 45),
            ("abs", 20),
            ("quads", 60),
            ("hamstrings", 75),
            ("glute-max", 100),
            ("gastrocnemius", 20),
        ],
    ),
    (
        "Seated Bicep Curl",
        &[("biceps", 100), ("brachialis", 45), ("forearm-flexors", 25)],
    ),
    (
        "Seated Calf Raise (Plate Loaded)",
        &[("gastrocnemius", 25), ("soleus", 100)],
    ),
    (
        "Seated Face Pull With Dual Cable",
        &[
            ("posterior-delts", 100),
            ("mid-traps", 60),
            ("lower-traps", 40),
            ("rhomboids", 55),
            ("biceps", 20),
            ("forearm-flexors", 15),
        ],
    ),
    (
        "Seated Revers grip Concentration Curl",
        &[
            ("biceps", 40),
            ("brachialis", 100),
            ("forearm-flexors", 15),
            ("forearm-extensors", 55),
        ],
    ),
    (
        "seated row",
        &[
            ("posterior-delts", 50),
            ("mid-traps", 100),
            ("lats", 90),
            ("rhomboids", 80),
            ("spinal-erectors", 15),
            ("biceps", 45),
            ("brachialis", 35),
            ("forearm-flexors", 30),
        ],
    ),
    (
        "Seated Shoulder Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 60),
            ("lower-traps", 20),
            ("upper-chest", 25),
            ("serratus-anterior", 20),
            ("triceps", 55),
        ],
    ),
    (
        "Shoulder Press",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 60),
            ("upper-traps", 15),
            ("lower-traps", 20),
            ("upper-chest", 20),
            ("serratus-anterior", 15),
            ("triceps", 55),
        ],
    ),
    (
        "Shoulder Press (Roc-It)",
        &[
            ("anterior-delts", 100),
            ("lateral-delts", 65),
            ("upper-traps", 20),
            ("lower-traps", 25),
            ("upper-chest", 30),
            ("serratus-anterior", 20),
            ("triceps", 55),
        ],
    ),
    (
        "Shrug",
        &[
            ("upper-traps", 100),
            ("mid-traps", 25),
            ("rhomboids", 15),
            ("forearm-flexors", 30),
        ],
    ),
    ("Side Crunch", &[("abs", 40), ("obliques", 100)]),
    (
        "Sissy Squat",
        &[
            ("abs", 15),
            ("hip-flexors", 20),
            ("quads", 100),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Sled 45° Leg Press",
        &[
            ("quads", 100),
            ("hamstrings", 25),
            ("adductors", 35),
            ("glute-max", 70),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Sled Hack Squat",
        &[
            ("quads", 100),
            ("hamstrings", 25),
            ("adductors", 40),
            ("glute-max", 70),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Smith Incline Bench Press",
        &[
            ("anterior-delts", 60),
            ("upper-chest", 100),
            ("mid-chest", 50),
            ("serratus-anterior", 10),
            ("triceps", 55),
        ],
    ),
    (
        "Smith Lateral Step-Up",
        &[
            ("quads", 100),
            ("hamstrings", 25),
            ("adductors", 30),
            ("glute-max", 80),
            ("glute-med", 60),
            ("gastrocnemius", 10),
            ("soleus", 10),
        ],
    ),
    (
        "Smith Shrug",
        &[
            ("upper-traps", 100),
            ("mid-traps", 20),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Smith Sprint Lunge",
        &[
            ("spinal-erectors", 15),
            ("abs", 10),
            ("quads", 100),
            ("hamstrings", 35),
            ("adductors", 30),
            ("glute-max", 85),
            ("glute-med", 40),
            ("gastrocnemius", 15),
        ],
    ),
    (
        "Smith Squat",
        &[
            ("spinal-erectors", 30),
            ("abs", 15),
            ("quads", 100),
            ("hamstrings", 30),
            ("adductors", 40),
            ("glute-max", 80),
            ("glute-med", 15),
            ("gastrocnemius", 10),
        ],
    ),
    (
        "Standing Cross-over High Reverse Fly",
        &[
            ("lateral-delts", 15),
            ("posterior-delts", 100),
            ("mid-traps", 45),
            ("lower-traps", 15),
            ("rhomboids", 45),
            ("forearm-extensors", 10),
        ],
    ),
    (
        "Standing Up Straight Crossovers",
        &[
            ("anterior-delts", 35),
            ("upper-chest", 20),
            ("mid-chest", 100),
            ("lower-chest", 45),
            ("serratus-anterior", 20),
            ("biceps", 10),
        ],
    ),
    (
        "Step-up",
        &[
            ("spinal-erectors", 10),
            ("quads", 100),
            ("hamstrings", 30),
            ("adductors", 25),
            ("glute-max", 85),
            ("glute-med", 45),
            ("gastrocnemius", 20),
            ("soleus", 15),
        ],
    ),
    (
        "Step-Up (Weighted)",
        &[
            ("spinal-erectors", 10),
            ("quads", 100),
            ("hamstrings", 35),
            ("adductors", 25),
            ("glute-max", 85),
            ("glute-med", 45),
            ("gastrocnemius", 15),
            ("soleus", 15),
        ],
    ),
    (
        "Straight Back Seated Row",
        &[
            ("posterior-delts", 45),
            ("mid-traps", 60),
            ("lower-traps", 20),
            ("lats", 100),
            ("rhomboids", 55),
            ("spinal-erectors", 15),
            ("biceps", 45),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Sumo Deadlift",
        &[
            ("upper-traps", 35),
            ("lats", 30),
            ("spinal-erectors", 70),
            ("forearm-flexors", 40),
            ("abs", 15),
            ("quads", 75),
            ("hamstrings", 60),
            ("adductors", 70),
            ("glute-max", 100),
            ("glute-med", 25),
        ],
    ),
    (
        "Super Low Row (Plate Machine)",
        &[
            ("posterior-delts", 40),
            ("mid-traps", 55),
            ("lats", 100),
            ("rhomboids", 50),
            ("spinal-erectors", 15),
            ("biceps", 50),
            ("brachialis", 30),
            ("forearm-flexors", 25),
        ],
    ),
    (
        "Torso Rotation (Machine)",
        &[("abs", 35), ("obliques", 100)],
    ),
    (
        "Triceps Dip",
        &[
            ("anterior-delts", 45),
            ("mid-chest", 30),
            ("lower-chest", 60),
            ("serratus-anterior", 10),
            ("triceps", 100),
        ],
    ),
    (
        "Triceps Dip (Assisted)",
        &[
            ("anterior-delts", 45),
            ("upper-chest", 10),
            ("mid-chest", 35),
            ("lower-chest", 55),
            ("serratus-anterior", 10),
            ("triceps", 100),
        ],
    ),
    (
        "Triceps Extension (Cable, One Arm)",
        &[
            ("triceps", 100),
            ("forearm-flexors", 10),
            ("forearm-extensors", 10),
        ],
    ),
    (
        "Triceps Press (Machine)",
        &[
            ("anterior-delts", 30),
            ("mid-chest", 25),
            ("lower-chest", 25),
            ("triceps", 100),
        ],
    ),
    (
        "Twin handle parallel grip lat pulldown",
        &[
            ("posterior-delts", 20),
            ("mid-traps", 30),
            ("lower-traps", 30),
            ("lats", 100),
            ("rhomboids", 25),
            ("biceps", 50),
            ("brachialis", 40),
            ("forearm-flexors", 20),
        ],
    ),
    (
        "Upright Row",
        &[
            ("anterior-delts", 30),
            ("lateral-delts", 100),
            ("posterior-delts", 15),
            ("upper-traps", 75),
            ("biceps", 30),
            ("brachialis", 25),
            ("forearm-flexors", 20),
        ],
    ),
    (
        "Vertical Pec Fly (Atlantis)",
        &[
            ("anterior-delts", 30),
            ("upper-chest", 35),
            ("mid-chest", 100),
            ("lower-chest", 35),
            ("serratus-anterior", 15),
            ("biceps", 10),
        ],
    ),
    (
        "Vertical Traction (Machine)",
        &[
            ("posterior-delts", 20),
            ("mid-traps", 30),
            ("lower-traps", 30),
            ("lats", 100),
            ("rhomboids", 25),
            ("biceps", 50),
            ("brachialis", 30),
            ("forearm-flexors", 20),
        ],
    ),
    (
        "Weighted Decline Sit-up",
        &[("abs", 100), ("obliques", 35), ("hip-flexors", 60)],
    ),
    (
        "Weighted Seated One Arm Reverse Wrist Curl",
        &[("forearm-extensors", 100)],
    ),
    ("Wrist Curl", &[("forearm-flexors", 100)]),
    ("Wrist Curls", &[("forearm-flexors", 100)]),
    ("Wrist Curls (Barbell)", &[("forearm-flexors", 100)]),
    (
        "Zercher Shrug",
        &[
            ("upper-traps", 100),
            ("mid-traps", 30),
            ("spinal-erectors", 25),
            ("biceps", 30),
            ("brachialis", 15),
            ("abs", 20),
        ],
    ),
];

/// The default weight rows for one exercise: the researched seed when the
/// table knows the name (source `seed`), otherwise tag-derived ratios
/// (source `derived`). An empty result means the exercise earns no muscle
/// credit and stays permanently unseeded (e.g. pure cardio).
pub(super) fn default_weights(
    name: &str,
    tags: &[(String, String)],
) -> Vec<(&'static str, u8, &'static str)> {
    if let Some((_, weights)) = SEED_WEIGHTS.iter().find(|(seeded, _)| *seeded == name) {
        return weights
            .iter()
            .map(|(muscle, ratio)| (*muscle, *ratio, "seed"))
            .collect();
    }
    derived_weights(tags)
        .into_iter()
        .map(|(muscle, ratio)| (muscle, ratio, "derived"))
        .collect()
}

/// Reconstruct the old primary/secondary split from taxonomy tags, then
/// expand each coarse muscle to its granular constituents at the same ratio.
/// Primaries are the coarse muscle tags claimed by any movement tag; the
/// remaining muscle tags are secondary. An exercise whose movements claim
/// nothing (or that has no movement tag) counts every tagged muscle as
/// primary — with no mover to compare against, "secondary" would be an
/// invention. This exactly reproduces the pre-weight ×2/×1 credit.
pub(super) fn derived_weights(tags: &[(String, String)]) -> Vec<(&'static str, u8)> {
    let muscles: Vec<&str> = tags
        .iter()
        .filter(|(kind, _)| kind == "muscle")
        .map(|(_, value)| value.as_str())
        .collect();
    let claimed: Vec<&str> = tags
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
    let has_primary = muscles.iter().any(|muscle| claimed.contains(muscle));

    // BTreeMap keyed by canonical position keeps output in display order and
    // lets a primary expansion win over a secondary one for a shared muscle.
    let mut ratios: BTreeMap<usize, (&'static str, u8)> = BTreeMap::new();
    for coarse in muscles {
        let ratio = if !has_primary || claimed.contains(&coarse) {
            100
        } else {
            50
        };
        for granular in muscle_taxonomy::expand_coarse_tag(coarse) {
            let entry = ratios
                .entry(muscle_taxonomy::muscle_order(granular))
                .or_insert((granular, 0));
            entry.1 = entry.1.max(ratio);
        }
    }
    ratios.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(kind: &str, value: &str) -> (String, String) {
        (kind.to_string(), value.to_string())
    }

    #[test]
    fn seed_table_is_canonical_and_unique() {
        let mut names: Vec<&str> = SEED_WEIGHTS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate seed exercise");
        for (name, weights) in SEED_WEIGHTS {
            assert!(!weights.is_empty(), "{name} seeds no muscles");
            let mut ids: Vec<&str> = weights.iter().map(|(id, _)| *id).collect();
            ids.sort_unstable();
            let muscle_total = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), muscle_total, "{name} repeats a muscle");
            for (muscle, ratio) in *weights {
                assert!(
                    muscle_taxonomy::canonical_muscle(muscle).is_some(),
                    "{name} seeds unknown muscle {muscle}"
                );
                assert!(
                    (1..=100).contains(ratio),
                    "{name} seeds {muscle} at out-of-range {ratio}"
                );
            }
        }
    }

    #[test]
    fn movement_rules_split_primary_from_secondary() {
        // Bench press: horizontal-push claims chest, so triceps rides at 50.
        let weights = derived_weights(&[
            tag("equipment", "barbell"),
            tag("movement", "horizontal-push"),
            tag("muscle", "chest"),
            tag("muscle", "triceps"),
        ]);
        assert_eq!(
            weights,
            vec![
                ("upper-chest", 100),
                ("mid-chest", 100),
                ("lower-chest", 100),
                ("triceps", 50),
            ]
        );

        // Hinge claims both of its co-emitted muscles as movers.
        let weights = derived_weights(&[
            tag("movement", "hinge"),
            tag("muscle", "glutes"),
            tag("muscle", "hamstrings"),
        ]);
        assert_eq!(
            weights,
            vec![("hamstrings", 100), ("glute-max", 100), ("glute-med", 100)]
        );
    }

    #[test]
    fn unclaimed_muscles_default_to_primary() {
        // No movement tag at all: the muscle list is the whole story.
        let weights = derived_weights(&[tag("muscle", "chest")]);
        assert_eq!(
            weights,
            vec![
                ("upper-chest", 100),
                ("mid-chest", 100),
                ("lower-chest", 100)
            ]
        );

        // A movement the table does not claim muscles for (e.g. carry)
        // behaves the same way.
        let weights = derived_weights(&[tag("movement", "carry"), tag("muscle", "forearms")]);
        assert_eq!(
            weights,
            vec![("forearm-flexors", 100), ("forearm-extensors", 100)]
        );
    }

    #[test]
    fn untagged_and_unknown_exercises_derive_nothing() {
        assert!(derived_weights(&[]).is_empty());
        assert!(derived_weights(&[tag("movement", "cardio")]).is_empty());
        let defaults = default_weights("Mystery Machine Press", &[]);
        assert!(defaults.is_empty());
    }

    #[test]
    fn seeded_names_win_over_derivation() {
        if let Some((name, expected)) = SEED_WEIGHTS.first() {
            let defaults = default_weights(name, &[tag("muscle", "chest")]);
            assert_eq!(defaults.len(), expected.len());
            assert!(defaults.iter().all(|(_, _, source)| *source == "seed"));
        }
    }
}
