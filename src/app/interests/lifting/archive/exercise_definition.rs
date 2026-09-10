//! Exercise definitions and deterministic, reviewable muscle suggestions.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::super::{
    filters::{EQUIPMENT, MOVEMENT_DETAILS, MOVEMENTS},
    muscle_taxonomy,
};
use super::snapshot::Snapshot;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub name: String,
    #[serde(default)]
    pub movements: Vec<String>,
    #[serde(default)]
    pub equipment: Vec<String>,
    #[serde(default)]
    pub weights: BTreeMap<String, u32>,
}

pub fn normalize_name(name: &str) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.len() > 200 || name.chars().any(char::is_control) {
        return Err("Enter an exercise name of at most 200 bytes.".into());
    }
    Ok(name)
}

impl Definition {
    pub fn validate(mut self) -> Result<Self, String> {
        self.name = normalize_name(&self.name)?;
        for (values, allowed) in [
            (
                &mut self.movements,
                MOVEMENTS
                    .iter()
                    .chain(MOVEMENT_DETAILS)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>(),
            ),
            (
                &mut self.equipment,
                EQUIPMENT.iter().map(|(id, _)| *id).collect(),
            ),
        ] {
            if values.len() > allowed.len()
                || values
                    .iter()
                    .any(|value| !allowed.contains(&value.as_str()))
            {
                return Err("Choose movement and equipment from the available options.".into());
            }
            values.sort();
            values.dedup();
        }
        let muscles: BTreeSet<_> = muscle_taxonomy::MUSCLE_GROUPS
            .iter()
            .flat_map(|(_, _, members)| members.iter().map(|(id, _)| *id))
            .collect();
        if self
            .weights
            .iter()
            .any(|(id, ratio)| !muscles.contains(id.as_str()) || *ratio > 100)
        {
            return Err("Muscle weights must be between 0 and 100.".into());
        }
        self.weights.retain(|_, value| *value > 0);
        Ok(self)
    }

    pub fn tags(&self) -> Vec<(String, String)> {
        let mut tags: BTreeSet<(String, String)> = self
            .movements
            .iter()
            .map(|value| ("movement".into(), value.clone()))
            .chain(
                self.equipment
                    .iter()
                    .map(|value| ("equipment".into(), value.clone())),
            )
            .collect();
        for muscle in self.weights.keys() {
            if let Some(coarse) = muscle_taxonomy::coarse_tag_for(muscle) {
                tags.insert(("muscle".into(), coarse.into()));
            }
        }
        tags.into_iter().collect()
    }

    pub fn from_snapshot(snapshot: &Snapshot, name: &str) -> Self {
        let tags = snapshot
            .exercise_tag_map()
            .get(name)
            .cloned()
            .unwrap_or_default();
        Self {
            name: name.into(),
            movements: tags
                .iter()
                .filter(|(kind, _)| kind == "movement")
                .map(|(_, value)| value.clone())
                .collect(),
            equipment: tags
                .iter()
                .filter(|(kind, _)| kind == "equipment")
                .map(|(_, value)| value.clone())
                .collect(),
            weights: snapshot
                .exercise_weight_map()
                .get(name)
                .into_iter()
                .flatten()
                .map(|(id, ratio)| ((*id).into(), *ratio))
                .collect(),
        }
    }
}

fn tokens(value: &str) -> BTreeSet<String> {
    value
        .to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect()
}

fn similarity(left: &BTreeSet<String>, right: &BTreeSet<String>) -> u32 {
    let union = left.union(right).count();
    if union == 0 {
        return 0;
    }
    (left.intersection(right).count() * 10_000 / union) as u32
}

/// Reference names, best first. No history-frequency bias or synthesized ratios.
pub fn references(snapshot: &Snapshot, definition: &Definition) -> Vec<String> {
    let movements: BTreeSet<_> = definition.movements.iter().cloned().collect();
    let equipment: BTreeSet<_> = definition.equipment.iter().cloned().collect();
    let name = tokens(&definition.name);
    let mut candidates = Vec::new();
    for candidate in snapshot.exercise_names() {
        if candidate == definition.name || !snapshot.is_weight_reference(&candidate) {
            continue;
        }
        let reference = Definition::from_snapshot(snapshot, &candidate);
        let candidate_movements = reference.movements.into_iter().collect();
        let movement_score = similarity(&movements, &candidate_movements);
        if movement_score == 0 || reference.weights.is_empty() {
            continue;
        }
        candidates.push((
            candidate.clone(),
            (
                movement_score,
                similarity(&equipment, &reference.equipment.into_iter().collect()),
                similarity(&name, &tokens(&candidate)),
            ),
        ));
    }
    candidates.sort_by(|(left, left_score), (right, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.to_lowercase().cmp(&right.to_lowercase()))
            .then_with(|| left.cmp(right))
    });
    candidates.into_iter().map(|(name, _)| name).collect()
}
