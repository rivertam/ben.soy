//! Fitness archive database IO: one coherent snapshot query and the atomic
//! import write path.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};

use anyhow::Context;
use benjisponge::data::{
    Db,
    fitness_models::{
        Exercise, ExerciseAlias, ExerciseMuscle, ExerciseTag, Interruption, LiftSet, Workout,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use surrealdb::types::SurrealValue;

use super::super::{muscle_seed, muscle_taxonomy};
use super::aliases::AliasMap;
use super::import::{IncomingExercise, IncomingTag, Payload, tag_signature};

/// The data version; 0 when the row does not exist yet.
pub async fn current_version(db: &Db) -> surrealdb::Result<i64> {
    let mut response = db
        .query("SELECT VALUE v FROM fitness_meta:version")
        .await?
        .check()?;
    let versions: Vec<i64> = response.take(0)?;
    Ok(versions.into_iter().next().unwrap_or(0))
}

#[derive(Deserialize, SurrealValue)]
struct ArchiveRows {
    version: i64,
    workouts: Vec<Workout>,
    sets: Vec<LiftSet>,
    aliases: Vec<ExerciseAlias>,
    tags: Vec<ExerciseTag>,
    weights: Vec<ExerciseMuscle>,
    interruptions: Vec<Interruption>,
}

/// The version and everything its snapshot needs from one read transaction.
/// Row order is irrelevant because the snapshot sorts.
pub async fn load_archive(
    db: &Db,
) -> anyhow::Result<(
    i64,
    Vec<Workout>,
    Vec<LiftSet>,
    Vec<ExerciseAlias>,
    Vec<ExerciseTag>,
    Vec<ExerciseMuscle>,
    Vec<Interruption>,
)> {
    let mut response = db
        .query(
            "RETURN {
                 version: (SELECT VALUE v FROM fitness_meta:version)[0] ?? 0,
                 workouts: (SELECT *, record::id(id) AS id FROM workouts),
                 sets: (SELECT *, record::id(id) AS id FROM sets),
                 aliases: (
                     SELECT alias_name, canonical_name FROM exercise_aliases
                 ),
                 tags: (
                     SELECT exercise_name, kind, value FROM exercise_tags
                 ),
                 weights: (
                     SELECT exercise_name, muscle, ratio_hundredths
                     FROM exercise_muscles
                 ),
                 interruptions: (
                     SELECT
                         record::id(id) AS id,
                         from_date,
                         to_date,
                         note,
                         emoji ?? '🤒' AS emoji,
                         updated_at
                     FROM fitness_interruptions
                 )
             };",
        )
        .await?
        .check()?;
    let rows: Option<ArchiveRows> = response.take(0)?;
    let rows = rows.context("fitness archive query returned no snapshot")?;
    Ok((
        rows.version,
        rows.workouts,
        rows.sets,
        rows.aliases,
        rows.tags,
        rows.weights,
        rows.interruptions,
    ))
}

/// All configured aliases. The table is intentionally small and alias
/// chains are forbidden by the admin write, so both snapshot and import code
/// can resolve it in memory without datastore-specific joins.
pub async fn exercise_aliases(db: &Db) -> surrealdb::Result<Vec<ExerciseAlias>> {
    let mut response = db
        .query("SELECT alias_name, canonical_name FROM exercise_aliases;")
        .await?
        .check()?;
    response.take(0)
}

/// Resolve every incoming exercise/set name before any idempotency or
/// taxonomy decision. `raw_exercise_name` remains untouched as provenance.
///
/// If a chunk contains both a canonical name and one of its aliases, the
/// canonical exercise's taxonomy wins. If it contains aliases only for an
/// already-known canonical exercise, stored taxonomy wins so an old source
/// spelling cannot silently retag the renamed movement. A clean database has
/// no target row yet, so the alias's shared classifier output seeds it.
async fn canonicalize_payload(db: &Db, payload: &Payload) -> surrealdb::Result<Payload> {
    let aliases = AliasMap::new(exercise_aliases(db).await?);
    if aliases.is_empty() {
        return Ok(payload.clone());
    }

    struct GroupedExercise {
        exercise: IncomingExercise,
        included_canonical_name: bool,
    }

    let mut grouped: BTreeMap<String, GroupedExercise> = BTreeMap::new();
    for incoming in &payload.exercises {
        let canonical_name = aliases.resolve(&incoming.name);
        let is_canonical = canonical_name == incoming.name;
        let replacement = IncomingExercise {
            name: canonical_name.clone(),
            tags: incoming.tags.clone(),
        };
        match grouped.entry(canonical_name) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(GroupedExercise {
                    exercise: replacement,
                    included_canonical_name: is_canonical,
                });
            }
            std::collections::btree_map::Entry::Occupied(mut entry) if is_canonical => {
                entry.get_mut().exercise = replacement;
                entry.get_mut().included_canonical_name = true;
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
    }

    let alias_only_targets: Vec<String> = grouped
        .iter()
        .filter(|(_, grouped)| !grouped.included_canonical_name)
        .map(|(name, _)| name.clone())
        .collect();
    if !alias_only_targets.is_empty() {
        #[derive(Deserialize, SurrealValue)]
        struct ExistingCanonicalRows {
            exercises: Vec<String>,
            tags: Vec<ExerciseTag>,
        }
        let mut response = db
            .query(
                "RETURN {
                     exercises: (
                         SELECT VALUE name FROM exercises WHERE name IN $names
                     ),
                     tags: (
                         SELECT exercise_name, kind, value FROM exercise_tags
                         WHERE exercise_name IN $names
                     )
                 };",
            )
            .bind(("names", alias_only_targets))
            .await?
            .check()?;
        let existing: Option<ExistingCanonicalRows> = response.take(0)?;
        let existing = existing.expect("RETURN always yields one canonical exercise scan");
        let existing_names: HashSet<String> = existing.exercises.into_iter().collect();
        let mut tags_by_exercise: HashMap<String, Vec<IncomingTag>> = HashMap::new();
        for tag in existing.tags {
            tags_by_exercise
                .entry(tag.exercise_name)
                .or_default()
                .push(IncomingTag {
                    kind: tag.kind,
                    value: tag.value,
                });
        }
        for (name, grouped) in &mut grouped {
            if !grouped.included_canonical_name && existing_names.contains(name) {
                grouped.exercise.tags = tags_by_exercise.remove(name).unwrap_or_default();
            }
        }
    }

    let mut sets = payload.sets.clone();
    for set in &mut sets {
        set.exercise_name = aliases.resolve(&set.exercise_name);
    }
    Ok(Payload {
        workouts: payload.workouts.clone(),
        exercises: grouped
            .into_values()
            .map(|grouped| grouped.exercise)
            .collect(),
        sets,
    })
}

/// The deterministic record key for an (exercise, muscle) pair: sha-256 of
/// both, newline-separated (exercise names are validated printable text, so
/// the separator is unambiguous). Mirrors `content::access::grant_id` — a hex
/// key sidesteps record-id escaping for names containing spaces and parens.
pub fn exercise_muscle_id(exercise_name: &str, muscle: &str) -> String {
    Sha256::digest(format!("{exercise_name}\n{muscle}"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One `muscles` vocabulary row, mirroring `muscle_taxonomy::MUSCLE_GROUPS`.
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue, PartialEq, Eq)]
struct MuscleRow {
    name: String,
    label: String,
    muscle_group: String,
    ordinal: i64,
}

/// One `exercise_muscles` row as written (reads drop `source`/`updated_at`).
#[derive(Clone, Debug, Deserialize, Serialize, SurrealValue)]
struct WeightWrite {
    id: String,
    exercise_name: String,
    muscle: String,
    ratio_hundredths: i64,
    source: String,
    updated_at: i64,
}

#[derive(Clone, Debug, Serialize, SurrealValue)]
struct WeightPair {
    exercise_name: String,
    muscle: String,
}

/// Reconcile the muscle vocabulary and seed default weights, insert-only at
/// exercise granularity: an exercise with any stored weight row — `seed`,
/// `derived`, or `admin` — is never touched, so hand-tuned ratios survive
/// every future reconcile the way hand-corrected taxonomy survives imports.
/// Exercises with no rows get the researched seed table, else ratios derived
/// from their stored taxonomy tags. Runs at the top of every snapshot load;
/// in steady state it reads, finds nothing missing, and writes nothing.
/// Deliberately no version bump: the same call builds the snapshot that
/// reads these rows.
pub async fn reconcile_muscle_weights(db: &Db, updated_at: i64) -> anyhow::Result<()> {
    let expected: Vec<MuscleRow> = muscle_taxonomy::MUSCLE_GROUPS
        .iter()
        .flat_map(|(group, _, members)| members.iter().map(move |(id, label)| (group, id, label)))
        .enumerate()
        .map(|(ordinal, (group, id, label))| MuscleRow {
            name: (*id).to_string(),
            label: (*label).to_string(),
            muscle_group: (*group).to_string(),
            ordinal: ordinal as i64,
        })
        .collect();
    let mut response = db
        .query("SELECT name, label, muscle_group, ordinal FROM muscles;")
        .await?
        .check()?;
    let mut stored_muscles: Vec<MuscleRow> = response.take(0)?;
    stored_muscles.sort_by_key(|row| row.ordinal);
    if stored_muscles != expected {
        db.query(
            "FOR $muscle IN $muscles {
                 UPSERT ONLY type::record('muscles', $muscle.name)
                     CONTENT $muscle RETURN NONE;
             };",
        )
        .bind(("muscles", expected))
        .await?
        .check()?;
    }

    let mut response = db
        .query(
            "RETURN {
                 exercises: (SELECT VALUE name FROM exercises),
                 weighted: (SELECT VALUE exercise_name FROM exercise_muscles)
             };",
        )
        .await?
        .check()?;
    #[derive(Deserialize, SurrealValue)]
    struct SeedScan {
        exercises: Vec<String>,
        weighted: Vec<String>,
    }
    let scan: Option<SeedScan> = response.take(0)?;
    let scan = scan.context("exercise/weight scan returned no result")?;
    let weighted: HashSet<String> = scan.weighted.into_iter().collect();
    let unweighted: Vec<String> = scan
        .exercises
        .into_iter()
        .filter(|name| !weighted.contains(name))
        .collect();
    if unweighted.is_empty() {
        return Ok(());
    }

    let mut response = db
        .query(
            "SELECT exercise_name, kind, value
             FROM exercise_tags
             WHERE exercise_name IN $names;",
        )
        .bind(("names", unweighted.clone()))
        .await?
        .check()?;
    let tag_rows: Vec<ExerciseTag> = response.take(0)?;
    let mut tags_by_exercise: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for tag in tag_rows {
        tags_by_exercise
            .entry(tag.exercise_name)
            .or_default()
            .push((tag.kind, tag.value));
    }

    let mut rows: Vec<WeightWrite> = Vec::new();
    for name in &unweighted {
        let tags = tags_by_exercise
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for (muscle, ratio, source) in muscle_seed::default_weights(name, tags) {
            rows.push(WeightWrite {
                id: exercise_muscle_id(name, muscle),
                exercise_name: name.clone(),
                muscle: muscle.to_string(),
                ratio_hundredths: i64::from(ratio),
                source: source.to_string(),
                updated_at,
            });
        }
    }
    if rows.is_empty() {
        return Ok(());
    }
    db.query(
        "BEGIN TRANSACTION;
         FOR $row IN $rows {
             UPSERT ONLY type::record('exercise_muscles', $row.id)
                 CONTENT $row RETURN NONE;
         };
         COMMIT TRANSACTION;",
    )
    .bind(("rows", rows))
    .await?
    .check()?;
    Ok(())
}

/// The stored weights for one exercise, with provenance — the exercise page's
/// read. Canonical muscle order is applied by the caller.
pub async fn exercise_weights(
    db: &Db,
    exercise_name: &str,
) -> surrealdb::Result<Vec<(String, i64, String)>> {
    #[derive(Deserialize, SurrealValue)]
    struct Row {
        muscle: String,
        ratio_hundredths: i64,
        source: String,
    }
    let mut response = db
        .query(
            "SELECT muscle, ratio_hundredths, source
             FROM exercise_muscles
             WHERE exercise_name = $name;",
        )
        .bind(("name", exercise_name.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = response.take(0)?;
    Ok(rows
        .into_iter()
        .map(|row| (row.muscle, row.ratio_hundredths, row.source))
        .collect())
}

/// Authoritatively replace one exercise's weights from the admin form.
/// Removed pairs are deleted with one `=` predicate each (compound-unique
/// table — an `IN [..]` delete can silently no-op, docs/surrealdb-notes.md),
/// kept pairs are UPSERTs of the same deterministic record, and the version
/// bumps so every reader recomputes. Returns the new version.
pub async fn replace_exercise_weights(
    db: &Db,
    exercise_name: &str,
    weights: &[(String, u32)],
    updated_at: i64,
) -> surrealdb::Result<i64> {
    let mut response = db
        .query("SELECT VALUE muscle FROM exercise_muscles WHERE exercise_name = $name;")
        .bind(("name", exercise_name.to_string()))
        .await?
        .check()?;
    let stored: Vec<String> = response.take(0)?;
    let kept: HashSet<&str> = weights.iter().map(|(muscle, _)| muscle.as_str()).collect();
    let removed: Vec<WeightPair> = stored
        .into_iter()
        .filter(|muscle| !kept.contains(muscle.as_str()))
        .map(|muscle| WeightPair {
            exercise_name: exercise_name.to_string(),
            muscle,
        })
        .collect();
    let rows: Vec<WeightWrite> = weights
        .iter()
        .map(|(muscle, ratio)| WeightWrite {
            id: exercise_muscle_id(exercise_name, muscle),
            exercise_name: exercise_name.to_string(),
            muscle: muscle.clone(),
            ratio_hundredths: i64::from(*ratio),
            source: "admin".to_string(),
            updated_at,
        })
        .collect();

    db.query(
        "BEGIN TRANSACTION;
         FOR $pair IN $removed {
             DELETE exercise_muscles
                 WHERE exercise_name = $pair.exercise_name
                     AND muscle = $pair.muscle;
         };
         FOR $row IN $rows {
             UPSERT ONLY type::record('exercise_muscles', $row.id)
                 CONTENT $row RETURN NONE;
         };
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("removed", removed))
    .bind(("rows", rows))
    .await?
    .check()?;
    current_version(db).await
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExerciseIdentityOutcome {
    Saved {
        canonical_name: String,
        version: i64,
        mutated: bool,
    },
    NotFound,
    Stale,
}

/// The exact identity change shown on the confirmation page. `merge_names`
/// includes the exercise being edited; any additional name owns stored
/// history that will be folded into `canonical_name` on confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExerciseIdentityPlan {
    pub current_name: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub merge_names: Vec<String>,
    pub added_aliases: Vec<String>,
    pub removed_aliases: Vec<String>,
    pub version: i64,
    pub mutated: bool,
}

#[derive(Clone, Debug, Serialize, SurrealValue)]
struct AliasWrite {
    alias_name: String,
    canonical_name: String,
    updated_at: i64,
}

/// Build the non-mutating preview used for both the warning page and the
/// eventual write. Naming another stored exercise — directly, through one of
/// its aliases, or as the new canonical name — deliberately pulls that
/// exercise into the merge. Its own aliases are carried forward so no old
/// importer spelling is stranded.
pub async fn plan_exercise_identity(
    db: &Db,
    current_name: &str,
    new_name: &str,
    requested_aliases: &[String],
) -> surrealdb::Result<Option<ExerciseIdentityPlan>> {
    #[derive(Deserialize, SurrealValue)]
    struct IdentityScan {
        version: i64,
        exercises: Vec<String>,
        aliases: Vec<ExerciseAlias>,
    }
    let mut response = db
        .query(
            "RETURN {
                 version: (SELECT VALUE v FROM fitness_meta:version)[0] ?? 0,
                 exercises: (SELECT VALUE name FROM exercises),
                 aliases: (
                     SELECT alias_name, canonical_name FROM exercise_aliases
                 )
             };",
        )
        .await?
        .check()?;
    let Some(scan) = response.take::<Option<IdentityScan>>(0)? else {
        return Ok(None);
    };
    let exercise_names: HashSet<String> = scan.exercises.into_iter().collect();
    let alias_map = AliasMap::new(scan.aliases.clone());
    let current_sources: HashSet<String> = exercise_names
        .iter()
        .filter(|name| alias_map.resolve(name) == current_name)
        .cloned()
        .collect();
    // A database-managed alias can make the snapshot's displayed canonical
    // name differ from every physical `exercises` row. The editor must begin
    // from those resolved source rows, not require a redundant target row.
    if current_sources.is_empty() {
        return Ok(None);
    }

    let alias_targets: HashMap<String, String> = scan
        .aliases
        .iter()
        .map(|row| (row.alias_name.clone(), row.canonical_name.clone()))
        .collect();
    let current_aliases: HashSet<String> = scan
        .aliases
        .iter()
        .filter(|row| row.canonical_name == current_name)
        .map(|row| row.alias_name.clone())
        .collect();

    let mut aliases: HashSet<String> = requested_aliases.iter().cloned().collect();
    aliases.remove(new_name);
    if new_name != current_name {
        aliases.insert(current_name.to_string());
    }

    let mut merge_names = current_sources;
    // Grow both sets to a fixed point. An explicit name may be a canonical
    // exercise or an alias owned by one; aliases belonging to newly merged
    // exercises are retained, and a retained alias that is itself a stale
    // canonical row pulls that row in too.
    loop {
        let before_merges = merge_names.len();
        let before_aliases = aliases.len();
        let candidates: Vec<String> = aliases
            .iter()
            .cloned()
            .chain(std::iter::once(new_name.to_string()))
            .collect();
        for candidate in candidates {
            if exercise_names.contains(&candidate) {
                merge_names.insert(candidate.clone());
            }
            if let Some(owner) = alias_targets.get(&candidate)
                && exercise_names.contains(owner)
            {
                merge_names.insert(owner.clone());
            }
        }
        for name in merge_names.clone() {
            if name != new_name {
                aliases.insert(name.clone());
            }
            if name != current_name {
                for row in scan.aliases.iter().filter(|row| row.canonical_name == name) {
                    aliases.insert(row.alias_name.clone());
                }
            }
        }
        aliases.remove(new_name);
        if before_merges == merge_names.len() && before_aliases == aliases.len() {
            break;
        }
    }

    let mut aliases: Vec<String> = aliases.into_iter().collect();
    aliases.sort_unstable_by(|a, b| {
        a.to_ascii_lowercase()
            .cmp(&b.to_ascii_lowercase())
            .then_with(|| a.cmp(b))
    });
    let final_aliases: HashSet<&str> = aliases.iter().map(String::as_str).collect();
    let mut added_aliases: Vec<String> = aliases
        .iter()
        .filter(|alias| !current_aliases.contains(alias.as_str()))
        .cloned()
        .collect();
    let mut removed_aliases: Vec<String> = current_aliases
        .iter()
        .filter(|alias| !final_aliases.contains(alias.as_str()))
        .cloned()
        .collect();
    added_aliases.sort_unstable();
    removed_aliases.sort_unstable();
    let mut merge_names: Vec<String> = merge_names.into_iter().collect();
    merge_names.sort_unstable();
    let mutated = new_name != current_name
        || merge_names.iter().any(|name| name != new_name)
        || !added_aliases.is_empty()
        || !removed_aliases.is_empty();

    Ok(Some(ExerciseIdentityPlan {
        current_name: current_name.to_string(),
        canonical_name: new_name.to_string(),
        aliases,
        merge_names,
        added_aliases,
        removed_aliases,
        version: scan.version,
        mutated,
    }))
}

/// Apply one previously reviewed identity plan atomically. Every normalized
/// set name moves to the selected canonical identity; raw source spelling is
/// deliberately untouched. The exercise being edited supplies taxonomy and
/// muscle weights when it has them, with the selected target and then another
/// merged exercise as fallbacks. Records are derived again from the combined
/// set history.
pub async fn replace_exercise_identity(
    db: &Db,
    reviewed: &ExerciseIdentityPlan,
    updated_at: i64,
) -> surrealdb::Result<ExerciseIdentityOutcome> {
    let Some(fresh) = plan_exercise_identity(
        db,
        &reviewed.current_name,
        &reviewed.canonical_name,
        &reviewed.aliases,
    )
    .await?
    else {
        return Ok(ExerciseIdentityOutcome::NotFound);
    };
    if fresh != *reviewed {
        return Ok(ExerciseIdentityOutcome::Stale);
    }
    if !reviewed.mutated {
        return Ok(ExerciseIdentityOutcome::Saved {
            canonical_name: reviewed.canonical_name.clone(),
            version: reviewed.version,
            mutated: false,
        });
    }

    #[derive(Deserialize, SurrealValue)]
    struct IdentityRows {
        tags: Vec<ExerciseTag>,
        weights: Vec<WeightWrite>,
    }
    let mut response = db
        .query(
            "RETURN {
                 tags: (
                     SELECT exercise_name, kind, value FROM exercise_tags
                     WHERE exercise_name IN $names
                 ),
                 weights: (
                     SELECT
                         crypto::sha256(string::concat(exercise_name, '\n', muscle)) AS id,
                         exercise_name,
                         muscle,
                         ratio_hundredths,
                         source,
                         updated_at
                     FROM exercise_muscles WHERE exercise_name IN $names
                 )
             };",
        )
        .bind(("names", reviewed.merge_names.clone()))
        .await?
        .check()?;
    let Some(rows) = response.take::<Option<IdentityRows>>(0)? else {
        return Ok(ExerciseIdentityOutcome::Stale);
    };

    let authority_order: Vec<&str> = std::iter::once(reviewed.current_name.as_str())
        .chain(
            (reviewed.canonical_name != reviewed.current_name)
                .then_some(reviewed.canonical_name.as_str()),
        )
        .chain(reviewed.merge_names.iter().map(String::as_str))
        .collect();
    let tag_authority = authority_order
        .iter()
        .find(|name| rows.tags.iter().any(|tag| tag.exercise_name == **name));
    let mut tags: Vec<ExerciseTag> = tag_authority
        .into_iter()
        .flat_map(|name| {
            rows.tags
                .iter()
                .filter(move |tag| tag.exercise_name == **name)
        })
        .map(|tag| ExerciseTag {
            exercise_name: reviewed.canonical_name.clone(),
            kind: tag.kind.clone(),
            value: tag.value.clone(),
        })
        .collect();
    tags.sort_unstable_by(|a, b| (&a.kind, &a.value).cmp(&(&b.kind, &b.value)));
    tags.dedup_by(|a, b| a.kind == b.kind && a.value == b.value);

    let weight_authority = authority_order.iter().find(|name| {
        rows.weights
            .iter()
            .any(|weight| weight.exercise_name == **name)
    });
    let weights: Vec<WeightWrite> = weight_authority
        .into_iter()
        .flat_map(|name| {
            rows.weights
                .iter()
                .filter(move |weight| weight.exercise_name == **name)
        })
        .map(|weight| WeightWrite {
            id: exercise_muscle_id(&reviewed.canonical_name, &weight.muscle),
            exercise_name: reviewed.canonical_name.clone(),
            muscle: weight.muscle.clone(),
            ratio_hundredths: weight.ratio_hundredths,
            source: weight.source.clone(),
            updated_at: weight.updated_at,
        })
        .collect();
    let kept_muscles: HashSet<&str> = weights
        .iter()
        .map(|weight| weight.muscle.as_str())
        .collect();
    let removed_weights: Vec<WeightPair> = rows
        .weights
        .iter()
        .filter(|weight| {
            weight.exercise_name != reviewed.canonical_name
                || !kept_muscles.contains(weight.muscle.as_str())
        })
        .map(|weight| WeightPair {
            exercise_name: weight.exercise_name.clone(),
            muscle: weight.muscle.clone(),
        })
        .collect();
    let aliases: Vec<AliasWrite> = reviewed
        .aliases
        .iter()
        .map(|alias_name| AliasWrite {
            alias_name: alias_name.clone(),
            canonical_name: reviewed.canonical_name.clone(),
            updated_at,
        })
        .collect();
    let new_exercise = Exercise {
        name: reviewed.canonical_name.clone(),
    };

    let mut response = db
        .query(
            "BEGIN TRANSACTION;
         FOR $source IN $merge_names {
             DELETE exercise_aliases WHERE canonical_name = $source RETURN NONE;
         };
         FOR $alias IN $aliases {
             DELETE exercise_aliases WHERE alias_name = $alias.alias_name RETURN NONE;
         };
         DELETE exercise_aliases WHERE alias_name = $new_name RETURN NONE;
         FOR $source IN $merge_names {
             UPDATE sets SET exercise_name = $new_name
                 WHERE exercise_name = $source RETURN NONE;
             DELETE exercise_tags WHERE exercise_name = $source RETURN NONE;
             IF $source != $new_name {
                 DELETE type::record('exercises', $source) RETURN NONE;
             };
         };
         UPSERT ONLY type::record('exercises', $new_name)
             CONTENT $new_exercise RETURN NONE;
         FOR $tag IN $tags {
             CREATE exercise_tags CONTENT $tag RETURN NONE;
         };
         FOR $pair IN $removed_weights {
             DELETE exercise_muscles
                 WHERE exercise_name = $pair.exercise_name
                     AND muscle = $pair.muscle RETURN NONE;
         };
         FOR $weight IN $weights {
             UPSERT ONLY type::record('exercise_muscles', $weight.id)
                 CONTENT $weight RETURN NONE;
         };
         FOR $alias IN $aliases {
             CREATE exercise_aliases CONTENT $alias RETURN NONE;
         };
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1 RETURN NONE;
             COMMIT TRANSACTION;",
        )
        .bind(("merge_names", reviewed.merge_names.clone()))
        .bind(("new_name", reviewed.canonical_name.clone()))
        .bind(("new_exercise", new_exercise))
        .bind(("tags", tags))
        .bind(("removed_weights", removed_weights))
        .bind(("weights", weights))
        .bind(("aliases", aliases))
        .await?;
    let mut errors: Vec<(usize, surrealdb::Error)> = response.take_errors().into_iter().collect();
    errors.sort_unstable_by_key(|(index, _)| *index);
    if !errors.is_empty() {
        for (index, error) in &errors {
            eprintln!("exercise identity transaction statement {index} failed: {error}");
        }
        let root_index = errors
            .iter()
            .position(|(_, error)| !error.to_string().contains("not executed"))
            .unwrap_or(0);
        return Err(errors.swap_remove(root_index).1);
    }

    Ok(ExerciseIdentityOutcome::Saved {
        canonical_name: reviewed.canonical_name.clone(),
        version: current_version(db).await?,
        mutated: true,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportOutcome {
    pub received: usize,
    pub added: usize,
    pub skipped: usize,
    pub version: i64,
    /// Whether anything was written (sets added or tags replaced) — the
    /// caller rebuilds the snapshot only then.
    pub mutated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualImportOutcome {
    Added,
    Duplicate,
    Conflict,
}

/// Create one owner-submitted workout without inheriting the CSV importer's
/// append semantics. An exact repeat is idempotent; the same deterministic
/// timestamp ID with any different workout/set content is a conflict.
///
/// Existing exercise taxonomy is left untouched. Only exercises that do not
/// exist yet are created and receive the parser's shared taxonomy.
pub async fn create_manual_workout(
    db: &Db,
    payload: &Payload,
    imported_at: i64,
) -> surrealdb::Result<ManualImportOutcome> {
    let Some(incoming_workout) = payload.workouts.first() else {
        return Ok(ManualImportOutcome::Conflict);
    };
    if payload.workouts.len() != 1 || incoming_workout.source != "manual" {
        return Ok(ManualImportOutcome::Conflict);
    }
    let payload = canonicalize_payload(db, payload).await?;
    let payload = &payload;

    // Preflight reads deliberately stay outside the write transaction so the
    // common path remains small. CREATE ONLY and the unique indexes close the
    // race atomically; if another writer wins between preflight and COMMIT,
    // re-read to classify an identical/timestamp-colliding workout, or retry
    // after a shared exercise/version conflict. SurrealDB 3.2.3 can surface a
    // transaction conflict as an earlier NotExecuted statement, so every
    // failed transaction gets the same conservative bounded treatment.
    const MAX_ATTEMPTS: usize = 3;
    for attempt in 1..=MAX_ATTEMPTS {
        if let Some(outcome) = existing_manual_outcome(db, payload).await? {
            return Ok(outcome);
        }
        match create_manual_workout_attempt(db, payload, imported_at).await {
            Ok(()) => return Ok(ManualImportOutcome::Added),
            Err(error) => {
                if let Ok(Some(outcome)) = existing_manual_outcome(db, payload).await {
                    return Ok(outcome);
                }
                if attempt == MAX_ATTEMPTS {
                    return Err(error);
                }
                tokio::time::sleep(Duration::from_millis(10 * attempt as u64)).await;
            }
        }
    }
    unreachable!("the bounded manual-import loop always returns")
}

async fn existing_manual_outcome(
    db: &Db,
    payload: &Payload,
) -> surrealdb::Result<Option<ManualImportOutcome>> {
    let incoming_workout = payload
        .workouts
        .first()
        .expect("manual payload was validated before lookup");
    let mut response = db
        .query(
            "SELECT *, record::id(id) AS id
             FROM workouts
             WHERE record::id(id) = $workout_id;",
        )
        .bind(("workout_id", incoming_workout.id.clone()))
        .await?
        .check()?;
    let mut existing_workouts: Vec<Workout> = response.take(0)?;
    if let Some(existing_workout) = existing_workouts.pop() {
        let mut response = db
            .query(
                "SELECT *, record::id(id) AS id
                 FROM sets
                 WHERE workout_id = $workout_id
                 ORDER BY ordinal ASC;",
            )
            .bind(("workout_id", incoming_workout.id.clone()))
            .await?
            .check()?;
        let existing_sets: Vec<LiftSet> = response.take(0)?;
        return Ok(Some(
            if same_manual_workout(&existing_workout, &existing_sets, payload) {
                ManualImportOutcome::Duplicate
            } else {
                ManualImportOutcome::Conflict
            },
        ));
    }
    Ok(None)
}

async fn create_manual_workout_attempt(
    db: &Db,
    payload: &Payload,
    imported_at: i64,
) -> surrealdb::Result<()> {
    let incoming_workout = payload
        .workouts
        .first()
        .expect("manual payload was validated before write");
    let exercise_names: Vec<String> = payload
        .exercises
        .iter()
        .map(|exercise| exercise.name.clone())
        .collect();
    let mut response = db
        .query("SELECT VALUE name FROM exercises WHERE name IN $names;")
        .bind(("names", exercise_names))
        .await?
        .check()?;
    let existing_exercises: HashSet<String> =
        response.take::<Vec<String>>(0)?.into_iter().collect();
    let missing_exercises: Vec<Exercise> = payload
        .exercises
        .iter()
        .filter(|exercise| !existing_exercises.contains(&exercise.name))
        .map(|exercise| Exercise {
            name: exercise.name.clone(),
        })
        .collect();
    let missing_names: HashSet<&str> = missing_exercises
        .iter()
        .map(|exercise| exercise.name.as_str())
        .collect();
    let added_tags: Vec<ExerciseTag> = payload
        .exercises
        .iter()
        .filter(|exercise| missing_names.contains(exercise.name.as_str()))
        .flat_map(|exercise| {
            exercise.tags.iter().map(|tag| ExerciseTag {
                exercise_name: exercise.name.clone(),
                kind: tag.kind.clone(),
                value: tag.value.clone(),
            })
        })
        .collect();

    let workouts = vec![Workout {
        id: incoming_workout.id.clone(),
        title: incoming_workout.title.clone(),
        raw_title: incoming_workout.raw_title.clone(),
        started_at_utc: incoming_workout.started_at_utc.clone(),
        started_at_local: incoming_workout.started_at_local.clone(),
        eastern_offset_minutes: incoming_workout.eastern_offset_minutes,
        duration_seconds: incoming_workout.duration_seconds,
        duration_suspicious: incoming_workout.duration_suspicious,
        notes: incoming_workout.notes.clone(),
        description: incoming_workout.description.clone(),
        source: incoming_workout.source.clone(),
        imported_at,
    }];
    let sets: Vec<LiftSet> = payload
        .sets
        .iter()
        .map(|set| LiftSet {
            id: set.id.clone(),
            workout_id: set.workout_id.clone(),
            exercise_name: set.exercise_name.clone(),
            raw_exercise_name: set.raw_exercise_name.clone(),
            ordinal: set.ordinal,
            exercise_note: set.exercise_note.clone(),
            superset_id: set.superset_id,
            weight_milli: set.weight_milli,
            weight_unit: set.weight_unit.clone(),
            reps: set.reps,
            effort_hundredths: set.effort_hundredths,
            distance_milli: set.distance_milli,
            set_time_seconds: set.set_time_seconds,
            set_type: set.set_type.clone(),
            incomplete: set.incomplete,
        })
        .collect();

    db.query(
        "BEGIN TRANSACTION;
         FOR $workout IN $workouts {
             CREATE ONLY type::record('workouts', $workout.id) CONTENT $workout;
         };
         FOR $exercise IN $exercises {
             CREATE ONLY type::record('exercises', $exercise.name) CONTENT $exercise;
         };
         FOR $tag IN $tags {
             CREATE exercise_tags CONTENT $tag;
         };
         FOR $set IN $sets {
             CREATE ONLY type::record('sets', $set.id) CONTENT $set;
         };
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("workouts", workouts))
    .bind(("exercises", missing_exercises))
    .bind(("tags", added_tags))
    .bind(("sets", sets))
    .await?
    .check()?;

    Ok(())
}

fn same_manual_workout(workout: &Workout, sets: &[LiftSet], payload: &Payload) -> bool {
    let Some(incoming) = payload.workouts.first() else {
        return false;
    };
    if payload.workouts.len() != 1
        || workout.id != incoming.id
        || workout.title != incoming.title
        || workout.raw_title != incoming.raw_title
        || workout.started_at_utc != incoming.started_at_utc
        || workout.started_at_local != incoming.started_at_local
        || workout.eastern_offset_minutes != incoming.eastern_offset_minutes
        || workout.duration_seconds != incoming.duration_seconds
        || workout.duration_suspicious != incoming.duration_suspicious
        || workout.notes != incoming.notes
        || workout.description != incoming.description
        || workout.source != incoming.source
        || sets.len() != payload.sets.len()
    {
        return false;
    }

    let mut stored: Vec<&LiftSet> = sets.iter().collect();
    stored.sort_unstable_by_key(|set| set.ordinal);
    let mut incoming: Vec<_> = payload.sets.iter().collect();
    incoming.sort_unstable_by_key(|set| set.ordinal);
    stored.into_iter().zip(incoming).all(|(stored, incoming)| {
        stored.id == incoming.id
            && stored.workout_id == incoming.workout_id
            && stored.exercise_name == incoming.exercise_name
            && stored.raw_exercise_name == incoming.raw_exercise_name
            && stored.ordinal == incoming.ordinal
            && stored.exercise_note == incoming.exercise_note
            && stored.superset_id == incoming.superset_id
            && stored.weight_milli == incoming.weight_milli
            && stored.weight_unit == incoming.weight_unit
            && stored.reps == incoming.reps
            && stored.effort_hundredths == incoming.effort_hundredths
            && stored.distance_milli == incoming.distance_milli
            && stored.set_time_seconds == incoming.set_time_seconds
            && stored.set_type == incoming.set_type
            && stored.incomplete == incoming.incomplete
    })
}

/// Validated fields for creating or updating an interruption.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterruptionWrite {
    pub from_date: String,
    /// `None` keeps the interruption open through today on the heatmap.
    pub to_date: Option<String>,
    pub note: String,
    pub emoji: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrittenInterruption {
    pub id: String,
    pub version: i64,
}

/// Create one annotate-only interruption and bump the fitness version.
pub async fn create_interruption(
    db: &Db,
    write: &InterruptionWrite,
    updated_at: i64,
) -> anyhow::Result<WrittenInterruption> {
    let id = interruption_id();
    let row = Interruption {
        id: id.clone(),
        from_date: write.from_date.clone(),
        to_date: write.to_date.clone(),
        note: write.note.clone(),
        emoji: write.emoji.clone(),
        updated_at,
    };
    db.query(
        "BEGIN TRANSACTION;
         CREATE ONLY type::record('fitness_interruptions', $row.id) CONTENT $row;
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("row", row))
    .await?
    .check()?;
    Ok(WrittenInterruption {
        id,
        version: current_version(db).await?,
    })
}

/// Update an existing interruption in place. `Ok(None)` when the id is absent.
pub async fn update_interruption(
    db: &Db,
    id: &str,
    write: &InterruptionWrite,
    updated_at: i64,
) -> anyhow::Result<Option<WrittenInterruption>> {
    if !is_interruption_id(id) {
        return Ok(None);
    }
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM type::record('fitness_interruptions', $id)",
        )
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    let existing: Vec<String> = response.take(0)?;
    if existing.is_empty() {
        return Ok(None);
    }

    db.query(
        "BEGIN TRANSACTION;
         UPDATE type::record('fitness_interruptions', $id) SET
             from_date = $from_date,
             to_date = $to_date,
             note = $note,
             emoji = $emoji,
             updated_at = $updated_at;
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("id", id.to_string()))
    .bind(("from_date", write.from_date.clone()))
    .bind(("to_date", write.to_date.clone()))
    .bind(("note", write.note.clone()))
    .bind(("emoji", write.emoji.clone()))
    .bind(("updated_at", updated_at))
    .await?
    .check()?;

    Ok(Some(WrittenInterruption {
        id: id.to_string(),
        version: current_version(db).await?,
    }))
}

/// Delete one interruption. `Ok(None)` when the id is absent.
pub async fn delete_interruption(db: &Db, id: &str) -> anyhow::Result<Option<WrittenInterruption>> {
    if !is_interruption_id(id) {
        return Ok(None);
    }
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM type::record('fitness_interruptions', $id)",
        )
        .bind(("id", id.to_string()))
        .await?
        .check()?;
    let existing: Vec<String> = response.take(0)?;
    if existing.is_empty() {
        return Ok(None);
    }

    db.query(
        "BEGIN TRANSACTION;
         DELETE type::record('fitness_interruptions', $id);
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("id", id.to_string()))
    .await?
    .check()?;

    Ok(Some(WrittenInterruption {
        id: id.to_string(),
        version: current_version(db).await?,
    }))
}

fn interruption_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn is_interruption_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// What a delete removed, for the caller's receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletedWorkout {
    pub workout_id: String,
    pub source: String,
    pub sets_deleted: usize,
    pub version: i64,
}

/// The one row the public path resolves to. `source` rides along because the
/// receipt reports it: deleting a `workout-data-csv` workout is undone by the
/// next `just sync-fitness`, and the caller deserves to be told.
#[derive(Deserialize, SurrealValue)]
struct DeletionTarget {
    id: String,
    source: String,
}

/// Delete one workout and its sets, addressed exactly the way the public API
/// addresses it — by Eastern local start plus offset, the pair `by_path`
/// matches on. `Ok(None)` means no workout is stored there.
///
/// Deliberately narrow: `sets`, the `workouts` row, and the version counter.
/// Exercise and tag rows are left alone even when this was an exercise's last
/// set. That is the same invariant the CSV reset relies on — the snapshot
/// never loads the `exercises` table, and every public count joins through
/// sets, so an orphan is invisible rather than harmless-but-visible. Keeping
/// them also preserves hand-corrected taxonomy across a delete-and-repaste.
///
/// Records need no cleanup: they are derived at snapshot build, so the
/// remaining history re-derives its own podium on the next rebuild.
pub async fn delete_workout_by_path(
    db: &Db,
    local: &str,
    offset_minutes: i32,
) -> surrealdb::Result<Option<DeletedWorkout>> {
    let mut response = db
        .query(
            "SELECT record::id(id) AS id, source
             FROM workouts
             WHERE started_at_local = $local AND eastern_offset_minutes = $offset;",
        )
        .bind(("local", local.to_string()))
        .bind(("offset", i64::from(offset_minutes)))
        .await?
        .check()?;
    let mut targets: Vec<DeletionTarget> = response.take(0)?;
    let Some(target) = targets.pop() else {
        return Ok(None);
    };

    // Counted before the delete rather than returned from it: `DELETE` yields
    // nothing by default, and `RETURN BEFORE` would hand back records whose
    // `id` is a record id the model cannot deserialize.
    let mut response = db
        .query("SELECT VALUE record::id(id) FROM sets WHERE workout_id = $workout_id;")
        .bind(("workout_id", target.id.clone()))
        .await?
        .check()?;
    let sets_deleted = response.take::<Vec<String>>(0)?.len();

    // `=` on the whole predicate, never `IN [..]`: on a table carrying a
    // compound unique index an `IN` delete can match nothing and still report
    // success (see docs/surrealdb-notes.md).
    db.query(
        "BEGIN TRANSACTION;
         DELETE sets WHERE workout_id = $workout_id;
         DELETE type::record('workouts', $workout_id);
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("workout_id", target.id.clone()))
    .await?
    .check()?;

    Ok(Some(DeletedWorkout {
        workout_id: target.id,
        source: target.source,
        sets_deleted,
        version: current_version(db).await?,
    }))
}

pub async fn apply_import(
    db: &Db,
    payload: &Payload,
    imported_at: i64,
) -> surrealdb::Result<ImportOutcome> {
    let payload = canonicalize_payload(db, payload).await?;
    let payload = &payload;
    let set_ids: Vec<String> = payload.sets.iter().map(|set| set.id.clone()).collect();
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM sets
             WHERE record::id(id) IN $set_ids;",
        )
        .bind(("set_ids", set_ids))
        .await?
        .check()?;
    let existing_sets: HashSet<String> = response.take::<Vec<String>>(0)?.into_iter().collect();
    let candidates: Vec<_> = payload
        .sets
        .iter()
        .filter(|set| !existing_sets.contains(&set.id))
        .collect();

    let exercise_names: Vec<String> = payload
        .exercises
        .iter()
        .map(|exercise| exercise.name.clone())
        .collect();
    let stored_tags: HashMap<String, Vec<IncomingTag>> = {
        let mut response = db
            .query(
                "SELECT exercise_name, kind, value
                 FROM exercise_tags
                 WHERE exercise_name IN $exercise_names;",
            )
            .bind(("exercise_names", exercise_names))
            .await?
            .check()?;
        let rows: Vec<ExerciseTag> = response.take(0)?;
        let mut by_exercise: HashMap<String, Vec<IncomingTag>> = HashMap::new();
        for row in rows {
            by_exercise
                .entry(row.exercise_name)
                .or_default()
                .push(IncomingTag {
                    kind: row.kind,
                    value: row.value,
                });
        }
        by_exercise
    };
    let changed_exercises: HashSet<&str> = payload
        .exercises
        .iter()
        .filter(|exercise| {
            let stored = stored_tags
                .get(&exercise.name)
                .map(Vec::as_slice)
                .unwrap_or_default();
            tag_signature(&exercise.tags) != tag_signature(stored)
        })
        .map(|exercise| exercise.name.as_str())
        .collect();

    let received = payload.sets.len();
    if candidates.is_empty() && changed_exercises.is_empty() {
        return Ok(ImportOutcome {
            received,
            added: 0,
            skipped: received,
            version: current_version(db).await?,
            mutated: false,
        });
    }

    let workout_ids: Vec<String> = payload
        .workouts
        .iter()
        .map(|workout| workout.id.clone())
        .collect();
    let mut response = db
        .query(
            "SELECT VALUE record::id(id)
             FROM workouts
             WHERE record::id(id) IN $workout_ids;",
        )
        .bind(("workout_ids", workout_ids))
        .await?
        .check()?;
    let existing_workouts: HashSet<String> = response.take::<Vec<String>>(0)?.into_iter().collect();
    let missing_workouts: Vec<Workout> = payload
        .workouts
        .iter()
        .filter(|workout| !existing_workouts.contains(&workout.id))
        .map(|workout| Workout {
            id: workout.id.clone(),
            title: workout.title.clone(),
            raw_title: workout.raw_title.clone(),
            started_at_utc: workout.started_at_utc.clone(),
            started_at_local: workout.started_at_local.clone(),
            eastern_offset_minutes: workout.eastern_offset_minutes,
            duration_seconds: workout.duration_seconds,
            duration_suspicious: workout.duration_suspicious,
            notes: workout.notes.clone(),
            description: workout.description.clone(),
            source: workout.source.clone(),
            imported_at,
        })
        .collect();

    let names: Vec<String> = payload
        .exercises
        .iter()
        .map(|exercise| exercise.name.clone())
        .collect();
    let mut response = db
        .query("SELECT VALUE name FROM exercises WHERE name IN $names;")
        .bind(("names", names))
        .await?
        .check()?;
    let existing_exercises: HashSet<String> =
        response.take::<Vec<String>>(0)?.into_iter().collect();
    let missing_exercises: Vec<Exercise> = payload
        .exercises
        .iter()
        .filter(|exercise| !existing_exercises.contains(&exercise.name))
        .map(|exercise| Exercise {
            name: exercise.name.clone(),
        })
        .collect();

    let mut removed_tags = Vec::new();
    let mut added_tags = Vec::new();
    for exercise in &payload.exercises {
        if !changed_exercises.contains(exercise.name.as_str()) {
            continue;
        }
        let stored = stored_tags
            .get(&exercise.name)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let incoming_keys: HashSet<_> = exercise
            .tags
            .iter()
            .map(|tag| (tag.kind.as_str(), tag.value.as_str()))
            .collect();
        for tag in stored {
            if !incoming_keys.contains(&(tag.kind.as_str(), tag.value.as_str())) {
                removed_tags.push(ExerciseTag {
                    exercise_name: exercise.name.clone(),
                    kind: tag.kind.clone(),
                    value: tag.value.clone(),
                });
            }
        }
        let stored_keys: HashSet<_> = stored
            .iter()
            .map(|tag| (tag.kind.as_str(), tag.value.as_str()))
            .collect();
        for tag in &exercise.tags {
            if !stored_keys.contains(&(tag.kind.as_str(), tag.value.as_str())) {
                added_tags.push(ExerciseTag {
                    exercise_name: exercise.name.clone(),
                    kind: tag.kind.clone(),
                    value: tag.value.clone(),
                });
            }
        }
    }

    let candidate_sets: Vec<LiftSet> = candidates
        .iter()
        .map(|set| LiftSet {
            id: set.id.clone(),
            workout_id: set.workout_id.clone(),
            exercise_name: set.exercise_name.clone(),
            raw_exercise_name: set.raw_exercise_name.clone(),
            ordinal: set.ordinal,
            exercise_note: set.exercise_note.clone(),
            superset_id: set.superset_id,
            weight_milli: set.weight_milli,
            weight_unit: set.weight_unit.clone(),
            reps: set.reps,
            effort_hundredths: set.effort_hundredths,
            distance_milli: set.distance_milli,
            set_time_seconds: set.set_time_seconds,
            set_type: set.set_type.clone(),
            incomplete: set.incomplete,
        })
        .collect();
    db.query(
        "BEGIN TRANSACTION;
         FOR $workout IN $workouts {
             CREATE ONLY type::record('workouts', $workout.id) CONTENT $workout;
         };
         FOR $exercise IN $exercises {
             CREATE ONLY type::record('exercises', $exercise.name) CONTENT $exercise;
         };
         FOR $tag IN $removed_tags {
             DELETE exercise_tags
                 WHERE exercise_name = $tag.exercise_name
                     AND kind = $tag.kind
                     AND value = $tag.value;
         };
         FOR $tag IN $added_tags {
             CREATE exercise_tags CONTENT $tag;
         };
         FOR $set IN $sets {
             CREATE ONLY type::record('sets', $set.id) CONTENT $set;
         };
         UPSERT fitness_meta:version SET k = 'version', v = (v ?? 0) + 1;
         COMMIT TRANSACTION;",
    )
    .bind(("workouts", missing_workouts))
    .bind(("exercises", missing_exercises))
    .bind(("removed_tags", removed_tags))
    .bind(("added_tags", added_tags))
    .bind(("sets", candidate_sets))
    .await?
    .check()?;

    let added = candidates.len();
    Ok(ImportOutcome {
        received,
        added,
        skipped: received - added,
        version: current_version(db).await?,
        mutated: true,
    })
}

#[cfg(test)]
mod tests {
    use surrealdb::engine::any;

    use super::*;
    use crate::app::interests::lifting::archive::import::{
        IncomingExercise, IncomingSet, IncomingWorkout,
    };

    const TEST_SCHEMA: &str = include_str!("../../../../schema.surql");

    async fn database() -> Db {
        let db = any::connect("mem://").await.unwrap();
        db.use_ns("fitness").use_db("fitness").await.unwrap();
        db.query(TEST_SCHEMA).await.unwrap().check().unwrap();
        db.query(
            "DEFINE INDEX exercise_aliases_alias_name
                 ON exercise_aliases FIELDS alias_name UNIQUE;
             DEFINE INDEX exercise_aliases_canonical_name
                 ON exercise_aliases FIELDS canonical_name;
             DEFINE INDEX exercise_tags_identity
                 ON exercise_tags FIELDS exercise_name, kind, value UNIQUE;
             DEFINE INDEX exercise_muscles_identity
                 ON exercise_muscles FIELDS exercise_name, muscle UNIQUE;",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
        db
    }

    fn payload() -> Payload {
        Payload {
            workouts: vec![IncomingWorkout {
                id: "fitness:2026-07-24T14:38:00".into(),
                title: "Arms".into(),
                raw_title: "Arms".into(),
                started_at_utc: "2026-07-24 14:38:00".into(),
                started_at_local: "2026-07-24 10:38:00".into(),
                eastern_offset_minutes: -240,
                duration_seconds: 720,
                duration_suspicious: false,
                notes: None,
                description: None,
                source: "manual".into(),
            }],
            exercises: vec![IncomingExercise {
                name: "Incline Bench Press".into(),
                tags: Vec::new(),
            }],
            sets: vec![IncomingSet {
                id: "fitness:2026-07-24T14:38:00:0001".into(),
                workout_id: "fitness:2026-07-24T14:38:00".into(),
                ordinal: 1,
                exercise_name: "Incline Bench Press".into(),
                raw_exercise_name: "Incline Bench Press".into(),
                exercise_note: None,
                superset_id: None,
                weight_milli: Some(135_000),
                weight_unit: "lbs".into(),
                reps: Some(6),
                effort_hundredths: Some(900),
                distance_milli: None,
                set_time_seconds: None,
                set_type: "NORMAL_SET".into(),
                incomplete: false,
            }],
        }
    }

    fn stored(payload: &Payload) -> (Workout, Vec<LiftSet>) {
        let workout = &payload.workouts[0];
        let set = &payload.sets[0];
        (
            Workout {
                id: workout.id.clone(),
                title: workout.title.clone(),
                raw_title: workout.raw_title.clone(),
                started_at_utc: workout.started_at_utc.clone(),
                started_at_local: workout.started_at_local.clone(),
                eastern_offset_minutes: workout.eastern_offset_minutes,
                duration_seconds: workout.duration_seconds,
                duration_suspicious: workout.duration_suspicious,
                notes: workout.notes.clone(),
                description: workout.description.clone(),
                source: workout.source.clone(),
                imported_at: 123,
            },
            vec![LiftSet {
                id: set.id.clone(),
                workout_id: set.workout_id.clone(),
                exercise_name: set.exercise_name.clone(),
                raw_exercise_name: set.raw_exercise_name.clone(),
                ordinal: set.ordinal,
                exercise_note: set.exercise_note.clone(),
                superset_id: set.superset_id,
                weight_milli: set.weight_milli,
                weight_unit: set.weight_unit.clone(),
                reps: set.reps,
                effort_hundredths: set.effort_hundredths,
                distance_milli: set.distance_milli,
                set_time_seconds: set.set_time_seconds,
                set_type: set.set_type.clone(),
                incomplete: set.incomplete,
            }],
        )
    }

    #[test]
    fn exact_manual_repeat_is_idempotent() {
        let payload = payload();
        let (workout, sets) = stored(&payload);
        assert!(same_manual_workout(&workout, &sets, &payload));
    }

    #[test]
    fn timestamp_collision_with_different_content_or_source_conflicts() {
        let payload = payload();
        let (workout, mut sets) = stored(&payload);
        sets[0].reps = Some(7);
        assert!(!same_manual_workout(&workout, &sets, &payload));

        let (mut workout, sets) = stored(&payload);
        workout.source = "workout-data-csv".into();
        assert!(!same_manual_workout(&workout, &sets, &payload));
    }

    #[tokio::test]
    async fn manual_import_resolves_alias_before_write_and_duplicate_check() {
        let db = database().await;
        db.query(
            "CREATE exercise_aliases CONTENT {
                 alias_name: 'Old Incline Press',
                 canonical_name: 'Incline Bench Press',
                 updated_at: 1
             };",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
        let mut payload = payload();
        payload.exercises[0].name = "Old Incline Press".into();
        payload.sets[0].exercise_name = "Old Incline Press".into();
        payload.sets[0].raw_exercise_name = "Old Incline Press".into();

        assert_eq!(
            create_manual_workout(&db, &payload, 123).await.unwrap(),
            ManualImportOutcome::Added
        );
        assert_eq!(
            create_manual_workout(&db, &payload, 456).await.unwrap(),
            ManualImportOutcome::Duplicate
        );

        let mut response = db
            .query(
                "RETURN {
                     stored: (SELECT VALUE exercise_name FROM sets)[0],
                     raw: (SELECT VALUE raw_exercise_name FROM sets)[0],
                     exercises: (SELECT VALUE name FROM exercises)
                 };",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let result: Option<serde_json::Value> = response.take(0).unwrap();
        let result = result.unwrap();
        assert_eq!(result["stored"], "Incline Bench Press");
        assert_eq!(result["raw"], "Old Incline Press");
        assert_eq!(
            result["exercises"],
            serde_json::json!(["Incline Bench Press"])
        );
    }

    #[tokio::test]
    async fn identity_save_renames_all_keys_and_keeps_old_name_as_alias() {
        let db = database().await;
        create_manual_workout(&db, &payload(), 123).await.unwrap();
        replace_exercise_weights(&db, "Incline Bench Press", &[("mid-chest".into(), 100)], 8)
            .await
            .unwrap();
        db.query(
            "CREATE exercise_aliases CONTENT {
                 alias_name: 'Old Incline Press',
                 canonical_name: 'Incline Bench Press',
                 updated_at: 1
             };",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        let plan = plan_exercise_identity(
            &db,
            "Incline Bench Press",
            "Incline Resurrection Press",
            &["Old Incline Press".into(), "Incline Press".into()],
        )
        .await
        .unwrap()
        .unwrap();
        assert!(plan.mutated);
        assert_eq!(plan.merge_names, vec!["Incline Bench Press"]);
        let outcome = replace_exercise_identity(&db, &plan, 9).await.unwrap();
        assert!(matches!(
            outcome,
            ExerciseIdentityOutcome::Saved {
                canonical_name,
                mutated: true,
                ..
            } if canonical_name == "Incline Resurrection Press"
        ));

        let mut response = db
            .query(
                "RETURN {
                     exercises: (SELECT VALUE name FROM exercises),
                     stored: (SELECT VALUE exercise_name FROM sets)[0],
                     raw: (SELECT VALUE raw_exercise_name FROM sets)[0],
                     tag_names: (SELECT VALUE exercise_name FROM exercise_tags),
                     weight_names: (SELECT VALUE exercise_name FROM exercise_muscles),
                     aliases: (
                         SELECT alias_name, canonical_name FROM exercise_aliases
                         ORDER BY alias_name ASC
                     )
                 };",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let result: Option<serde_json::Value> = response.take(0).unwrap();
        let result = result.unwrap();
        assert_eq!(
            result["exercises"],
            serde_json::json!(["Incline Resurrection Press"])
        );
        assert_eq!(result["stored"], "Incline Resurrection Press");
        assert_eq!(result["raw"], "Incline Bench Press");
        assert_eq!(
            result["weight_names"],
            serde_json::json!(["Incline Resurrection Press"])
        );
        assert_eq!(result["aliases"].as_array().unwrap().len(), 3);
        assert!(
            result["aliases"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| { row["canonical_name"] == "Incline Resurrection Press" })
        );
    }

    #[tokio::test]
    async fn identity_editor_accepts_a_logical_canonical_without_a_physical_row() {
        let db = database().await;
        create_manual_workout(&db, &payload(), 123).await.unwrap();
        db.query(
            "CREATE exercise_aliases CONTENT {
                 alias_name: 'Incline Bench Press',
                 canonical_name: 'Incline Resurrection Press',
                 updated_at: 1
             };",
        )
        .await
        .unwrap()
        .check()
        .unwrap();

        let plan = plan_exercise_identity(
            &db,
            "Incline Resurrection Press",
            "Incline Resurrection Lift",
            &["Incline Bench Press".into()],
        )
        .await
        .unwrap()
        .expect("the displayed canonical resolves through its physical alias row");
        assert_eq!(plan.merge_names, vec!["Incline Bench Press"]);
        assert!(plan.mutated);

        assert!(matches!(
            replace_exercise_identity(&db, &plan, 9).await.unwrap(),
            ExerciseIdentityOutcome::Saved {
                canonical_name,
                mutated: true,
                ..
            } if canonical_name == "Incline Resurrection Lift"
        ));
        let mut response = db
            .query(
                "RETURN {
                     exercise: (SELECT VALUE name FROM exercises)[0],
                     stored: (SELECT VALUE exercise_name FROM sets)[0],
                     raw: (SELECT VALUE raw_exercise_name FROM sets)[0],
                     aliases: (SELECT VALUE alias_name FROM exercise_aliases ORDER BY alias_name)
                 };",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let result: Option<serde_json::Value> = response.take(0).unwrap();
        let result = result.unwrap();
        assert_eq!(result["exercise"], "Incline Resurrection Lift");
        assert_eq!(result["stored"], "Incline Resurrection Lift");
        assert_eq!(result["raw"], "Incline Bench Press");
        assert_eq!(
            result["aliases"],
            serde_json::json!(["Incline Bench Press", "Incline Resurrection Press"])
        );
    }

    #[tokio::test]
    async fn reviewed_alias_can_merge_another_exercises_history() {
        let db = database().await;
        create_manual_workout(&db, &payload(), 123).await.unwrap();

        let mut overhead = payload();
        overhead.workouts[0].id = "fitness:2026-07-25T14:38:00".into();
        overhead.workouts[0].started_at_utc = "2026-07-25 14:38:00".into();
        overhead.workouts[0].started_at_local = "2026-07-25 10:38:00".into();
        overhead.exercises[0].name = "Barbell Overhead Press".into();
        overhead.sets[0].id = "fitness:2026-07-25T14:38:00:0001".into();
        overhead.sets[0].workout_id = overhead.workouts[0].id.clone();
        overhead.sets[0].exercise_name = "Barbell Overhead Press".into();
        overhead.sets[0].raw_exercise_name = "Barbell Overhead Press".into();
        create_manual_workout(&db, &overhead, 124).await.unwrap();

        db.query(
            "CREATE exercise_tags CONTENT {
                 exercise_name: 'Incline Bench Press', kind: 'muscle', value: 'chest'
             };
             CREATE exercise_tags CONTENT {
                 exercise_name: 'Barbell Overhead Press', kind: 'muscle', value: 'shoulders'
             };
             CREATE exercise_aliases CONTENT {
                 alias_name: 'Strict Press',
                 canonical_name: 'Barbell Overhead Press',
                 updated_at: 1
             };",
        )
        .await
        .unwrap()
        .check()
        .unwrap();
        replace_exercise_weights(&db, "Incline Bench Press", &[("mid-chest".into(), 100)], 8)
            .await
            .unwrap();
        replace_exercise_weights(
            &db,
            "Barbell Overhead Press",
            &[("anterior-delts".into(), 100)],
            8,
        )
        .await
        .unwrap();

        let plan = plan_exercise_identity(
            &db,
            "Incline Bench Press",
            "Incline Bench Press",
            &["Barbell Overhead Press".into()],
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            plan.merge_names,
            vec!["Barbell Overhead Press", "Incline Bench Press"]
        );
        assert_eq!(
            plan.added_aliases,
            vec!["Barbell Overhead Press", "Strict Press"]
        );
        assert!(plan.mutated);

        assert!(matches!(
            replace_exercise_identity(&db, &plan, 9).await.unwrap(),
            ExerciseIdentityOutcome::Saved { mutated: true, .. }
        ));
        let mut response = db
            .query(
                "RETURN {
                     exercises: (SELECT VALUE name FROM exercises),
                     stored: (SELECT VALUE exercise_name FROM sets ORDER BY id ASC),
                     raw: (SELECT VALUE raw_exercise_name FROM sets ORDER BY id ASC),
                     tags: (SELECT VALUE value FROM exercise_tags),
                     weights: (
                         SELECT muscle, ratio_hundredths FROM exercise_muscles
                     ),
                     aliases: (
                         SELECT alias_name, canonical_name FROM exercise_aliases
                         ORDER BY alias_name ASC
                     )
                 };",
            )
            .await
            .unwrap()
            .check()
            .unwrap();
        let result: Option<serde_json::Value> = response.take(0).unwrap();
        let result = result.unwrap();
        assert_eq!(
            result["exercises"],
            serde_json::json!(["Incline Bench Press"])
        );
        assert_eq!(
            result["stored"],
            serde_json::json!(["Incline Bench Press", "Incline Bench Press"])
        );
        assert_eq!(
            result["raw"],
            serde_json::json!(["Incline Bench Press", "Barbell Overhead Press"])
        );
        assert_eq!(result["tags"], serde_json::json!(["chest"]));
        assert_eq!(result["weights"].as_array().unwrap().len(), 1);
        assert_eq!(result["weights"][0]["muscle"], "mid-chest");
        assert_eq!(
            result["aliases"],
            serde_json::json!([
                {
                    "alias_name": "Barbell Overhead Press",
                    "canonical_name": "Incline Bench Press"
                },
                {
                    "alias_name": "Strict Press",
                    "canonical_name": "Incline Bench Press"
                }
            ])
        );
    }

    #[tokio::test]
    async fn reviewed_identity_plan_goes_stale_when_fitness_version_changes() {
        let db = database().await;
        create_manual_workout(&db, &payload(), 123).await.unwrap();
        let plan = plan_exercise_identity(
            &db,
            "Incline Bench Press",
            "Incline Press",
            &["Incline Bench Press".into()],
        )
        .await
        .unwrap()
        .unwrap();

        db.query("UPDATE fitness_meta:version SET v += 1;")
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            replace_exercise_identity(&db, &plan, 9).await.unwrap(),
            ExerciseIdentityOutcome::Stale
        );

        let mut response = db
            .query("SELECT VALUE name FROM exercises;")
            .await
            .unwrap()
            .check()
            .unwrap();
        assert_eq!(
            response.take::<Vec<String>>(0).unwrap(),
            vec!["Incline Bench Press"]
        );
    }
}
