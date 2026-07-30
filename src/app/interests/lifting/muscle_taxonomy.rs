//! The site's granular muscle vocabulary.
//!
//! One display group holds several granular muscles; weights, load bars, and
//! the body map all speak granular ids, while headers and the coarse
//! `muscle` tag facet stay at group scale. This module is the single Rust
//! source of truth — the `muscles`/`exercise_muscles` ASSERT lists in
//! `src/schema.surql` mirror it and a test keeps them aligned.

/// `(group id, group label, [(muscle id, muscle label)])`.
pub(super) type MuscleGroup = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

/// Display groups in display order. The flattened muscle order defines the
/// site-wide canonical muscle order.
pub(super) const MUSCLE_GROUPS: &[MuscleGroup] = &[
    (
        "shoulders",
        "shoulders",
        &[
            ("anterior-delts", "anterior delts"),
            ("lateral-delts", "lateral delts"),
            ("posterior-delts", "posterior delts"),
        ],
    ),
    (
        "traps",
        "traps",
        &[
            ("upper-traps", "upper traps"),
            ("mid-traps", "mid traps"),
            ("lower-traps", "lower traps"),
        ],
    ),
    (
        "back",
        "back",
        &[
            ("lats", "lats"),
            ("rhomboids", "rhomboids"),
            ("spinal-erectors", "spinal erectors"),
        ],
    ),
    (
        "chest",
        "chest",
        &[
            ("upper-chest", "upper chest"),
            ("mid-chest", "mid chest"),
            ("lower-chest", "lower chest"),
            ("serratus-anterior", "serratus anterior"),
        ],
    ),
    (
        "arms",
        "arms",
        &[
            ("biceps", "biceps"),
            ("brachialis", "brachialis"),
            ("triceps", "triceps"),
            ("forearm-flexors", "forearm flexors"),
            ("forearm-extensors", "forearm extensors"),
        ],
    ),
    (
        "core",
        "core",
        &[
            ("abs", "abs"),
            ("obliques", "obliques"),
            ("hip-flexors", "hip flexors"),
        ],
    ),
    (
        "legs",
        "legs",
        &[
            ("quads", "quads"),
            ("hamstrings", "hamstrings"),
            ("adductors", "adductors"),
        ],
    ),
    (
        "glutes",
        "glutes",
        &[("glute-max", "glute max"), ("glute-med", "glute med")],
    ),
    (
        "calves",
        "calves",
        &[("gastrocnemius", "gastrocnemius"), ("soleus", "soleus")],
    ),
];

/// Every granular muscle as `(id, label)` in canonical (display) order.
pub(super) fn muscles() -> impl Iterator<Item = (&'static str, &'static str)> {
    MUSCLE_GROUPS
        .iter()
        .flat_map(|(_, _, members)| members.iter().copied())
}

/// Map a stored value onto the canonical vocabulary; `None` for strangers.
pub(super) fn canonical_muscle(value: &str) -> Option<&'static str> {
    muscles().find_map(|(id, _)| (id == value).then_some(id))
}

pub(super) fn muscle_label(id: &str) -> Option<&'static str> {
    muscles().find_map(|(candidate, label)| (candidate == id).then_some(label))
}

/// Canonical position, used to sort mixed muscle lists into display order.
pub(super) fn muscle_order(id: &str) -> usize {
    muscles()
        .position(|(candidate, _)| candidate == id)
        .unwrap_or(usize::MAX)
}

/// The coarse `muscle` tag value whose `/lifting/log` facet best covers a
/// granular muscle. Tags deliberately stay at the original 13-value scale
/// (`filters::MUSCLES`), so load rows and legends link through this mapping.
pub(super) fn coarse_tag_for(muscle_id: &str) -> Option<&'static str> {
    let coarse = match muscle_id {
        "anterior-delts" | "lateral-delts" | "posterior-delts" => "shoulders",
        "upper-traps" | "mid-traps" | "lower-traps" => "traps",
        "lats" | "rhomboids" | "spinal-erectors" => "back",
        "upper-chest" | "mid-chest" | "lower-chest" | "serratus-anterior" => "chest",
        "biceps" | "brachialis" => "biceps",
        "triceps" => "triceps",
        "forearm-flexors" | "forearm-extensors" => "forearms",
        "abs" | "obliques" | "hip-flexors" => "core",
        "quads" => "quads",
        "hamstrings" => "hamstrings",
        "adductors" => "adductors",
        "glute-max" | "glute-med" => "glutes",
        "gastrocnemius" | "soleus" => "calves",
        _ => return None,
    };
    Some(coarse)
}

/// The granular constituents a coarse `muscle` tag expands to. Used only by
/// the derived-weights fallback for exercises the seed table doesn't know.
pub(super) fn expand_coarse_tag(coarse: &str) -> &'static [&'static str] {
    match coarse {
        "shoulders" => &["anterior-delts", "lateral-delts", "posterior-delts"],
        "traps" => &["upper-traps", "mid-traps", "lower-traps"],
        // The old "back" tag rides pull movements; erectors earn credit only
        // through researched seeds, never through this blanket expansion.
        "back" => &["lats", "rhomboids"],
        "chest" => &["upper-chest", "mid-chest", "lower-chest"],
        "biceps" => &["biceps", "brachialis"],
        "triceps" => &["triceps"],
        "forearms" => &["forearm-flexors", "forearm-extensors"],
        "core" => &["abs", "obliques"],
        "quads" => &["quads"],
        "hamstrings" => &["hamstrings"],
        "adductors" => &["adductors"],
        "glutes" => &["glute-max", "glute-med"],
        "calves" => &["gastrocnemius", "soleus"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_is_unique_and_self_consistent() {
        let all: Vec<_> = muscles().collect();
        assert_eq!(all.len(), 28);
        let mut ids: Vec<_> = all.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "duplicate muscle id");
        for (id, label) in &all {
            assert_eq!(canonical_muscle(id), Some(*id));
            assert_eq!(muscle_label(id), Some(*label));
            let coarse = coarse_tag_for(id).expect("every muscle links a coarse tag");
            assert!(
                super::super::filters::MUSCLES
                    .iter()
                    .any(|(tag, _)| *tag == coarse),
                "{id} maps to unknown coarse tag {coarse}"
            );
        }
        assert_eq!(
            canonical_muscle("chest"),
            None,
            "coarse ids are not muscles"
        );
    }

    #[test]
    fn every_coarse_tag_expands_to_canonical_muscles() {
        for (coarse, _) in super::super::filters::MUSCLES {
            let expansion = expand_coarse_tag(coarse);
            assert!(
                !expansion.is_empty(),
                "coarse tag {coarse} expands to nothing"
            );
            for id in expansion {
                assert!(
                    canonical_muscle(id).is_some(),
                    "{coarse} expands to unknown muscle {id}"
                );
            }
        }
    }

    /// Extract the quoted values of the `ASSERT $value IN [...]` list that
    /// follows `marker` in the schema source.
    fn assert_list(schema: &str, marker: &str) -> Vec<String> {
        let after = &schema[schema
            .find(marker)
            .unwrap_or_else(|| panic!("schema.surql lost the field definition {marker:?}"))
            + marker.len()..];
        let start = after.find('[').expect("ASSERT IN list opens") + 1;
        let end = after.find(']').expect("ASSERT IN list closes");
        after[start..end]
            .split(',')
            .map(|value| value.trim().trim_matches('\'').to_string())
            .filter(|value| !value.is_empty())
            .collect()
    }

    /// Both ASSERT lists must enumerate exactly this vocabulary — a muscle
    /// present in only one of them would pass `just check` and then fail
    /// the schema ASSERT at runtime on the first reconcile that seeds it.
    #[test]
    fn schema_assert_lists_match_the_vocabulary() {
        let schema = include_str!("../../../schema.surql");
        let expected: std::collections::BTreeSet<String> =
            muscles().map(|(id, _)| id.to_string()).collect();
        for marker in [
            "DEFINE FIELD OVERWRITE name ON muscles TYPE string",
            "DEFINE FIELD OVERWRITE muscle ON exercise_muscles TYPE string",
        ] {
            let listed: std::collections::BTreeSet<String> =
                assert_list(schema, marker).into_iter().collect();
            assert_eq!(listed, expected, "ASSERT list after {marker:?} drifted");
        }
        let expected_groups: std::collections::BTreeSet<String> = MUSCLE_GROUPS
            .iter()
            .map(|(group, ..)| (*group).to_string())
            .collect();
        let listed_groups: std::collections::BTreeSet<String> = assert_list(
            schema,
            "DEFINE FIELD OVERWRITE muscle_group ON muscles TYPE string",
        )
        .into_iter()
        .collect();
        assert_eq!(
            listed_groups, expected_groups,
            "muscle_group ASSERT drifted"
        );
    }
}
