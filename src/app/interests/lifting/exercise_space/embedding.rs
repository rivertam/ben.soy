//! Build exact UMAP neighborhoods from muscle mixtures; tags label the map.
mod vendor;

use fitness_entry_core::ExerciseGuide;
use serde::Serialize;
use topcoat::asset::{Asset, asset};

use super::super::filters::{MOVEMENT_DETAILS, MOVEMENTS};

pub(super) const WORKER_JS: Asset = asset!("./embedding/worker.js");
pub(super) const LIBRARY_JS: Asset = vendor::JS;
const NEIGHBORHOOD: usize = 25;
const DISTANCE_SCALE: u16 = 10_000;

#[derive(Serialize)]
pub(super) struct Embedding {
    features: Vec<Vec<f64>>,
    distance_scale: u16,
    graph: Graph,
}

#[derive(Serialize)]
struct Graph {
    indices: Vec<Vec<usize>>,
    distances: Vec<Vec<f64>>,
    // Upper triangle, row-major, without the diagonal. Four decimal places
    // keep the whole-catalog distance constraints compact in the page data.
    pair_distances: Vec<u16>,
}

pub(super) fn patterns(exercise: &ExerciseGuide) -> Vec<String> {
    let mut patterns: Vec<_> = exercise
        .movements
        .iter()
        .filter(|id| {
            MOVEMENTS
                .iter()
                .chain(MOVEMENT_DETAILS)
                .any(|(known, _)| known == id)
        })
        .cloned()
        .collect();
    patterns.sort();
    patterns.dedup();
    patterns
}

pub(super) fn label(pattern: &str) -> &'static str {
    match pattern {
        "squat-type" => "squats",
        "hinge" => "hinges",
        "core" => "trunk work",
        "elbow-extension" => "triceps extensions",
        _ => MOVEMENTS
            .iter()
            .chain(MOVEMENT_DETAILS)
            .find_map(|(id, label)| (*id == pattern).then_some(*label))
            .unwrap_or(""),
    }
}

// Scale to each exercise's strongest muscle, not its total involvement.
// Adding other muscles must not dilute a shared support muscle. Square roots
// retain secondary involvement while letting vector lengths vary naturally.
fn muscle_features(profile: &[f64]) -> Vec<f64> {
    let peak = profile.iter().copied().fold(0.0, f64::max);
    profile
        .iter()
        .map(|value| {
            if peak > 0.0 {
                (value / peak).sqrt()
            } else {
                0.0
            }
        })
        .collect()
}

fn distance(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum::<f64>()
        .sqrt()
}

pub(super) fn build(profiles: &[Vec<f64>]) -> Embedding {
    let features: Vec<_> = profiles
        .iter()
        .map(|profile| muscle_features(profile))
        .collect();
    let graph = {
        let mut indices = Vec::new();
        let mut distances = Vec::new();
        let mut pair_distances = Vec::new();
        for index in 0..features.len() {
            let mut neighbors: Vec<_> = (0..features.len())
                .map(|other| (other, distance(&features[index], &features[other])))
                .collect();
            pair_distances.extend(
                neighbors
                    .iter()
                    .skip(index + 1)
                    .map(|(_, distance)| (distance * f64::from(DISTANCE_SCALE)).round() as u16),
            );
            neighbors.sort_by(|(left, a), (right, b)| {
                a.total_cmp(b)
                    .then_with(|| (*left != index).cmp(&(*right != index)))
                    .then_with(|| left.cmp(right))
            });
            neighbors.truncate(NEIGHBORHOOD.min(features.len().saturating_sub(1)));
            indices.push(neighbors.iter().map(|(index, _)| *index).collect());
            distances.push(neighbors.iter().map(|(_, distance)| *distance).collect());
        }
        Graph {
            indices,
            distances,
            pair_distances,
        }
    };
    Embedding {
        features,
        distance_scale: DISTANCE_SCALE,
        graph,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secondary_muscle_contributions_influence_the_layout() {
        // A secondary 25% weight becomes half the primary's feature value.
        let features = muscle_features(&[100.0, 25.0]);
        assert!((features[1] / features[0] - 0.5).abs() < 1e-12);
        assert_eq!(features, muscle_features(&[40.0, 10.0]));
        assert_eq!(muscle_features(&[0.0, 0.0]), [0.0, 0.0]);
    }

    #[test]
    fn compound_involvement_does_not_dilute_shared_support_or_force_unit_lengths() {
        let narrow = muscle_features(&[100.0, 25.0, 0.0]);
        let compound = muscle_features(&[100.0, 25.0, 100.0]);
        assert_eq!(narrow[..2], compound[..2]);
        let squared_length =
            |profile: &[f64]| profile.iter().map(|value| value * value).sum::<f64>();
        assert!(squared_length(&compound) > squared_length(&narrow));
    }

    #[test]
    fn distances_for_the_full_muscle_vocabulary_fit_in_the_packed_format() {
        let profiles = vec![
            (0..28).map(|i| if i < 14 { 1.0 } else { 0.0 }).collect(),
            (0..28).map(|i| if i >= 14 { 1.0 } else { 0.0 }).collect(),
        ];
        let embedding = build(&profiles);
        let expected = 28_f64.sqrt();
        assert!(expected * f64::from(DISTANCE_SCALE) < f64::from(u16::MAX));
        assert!(
            (f64::from(embedding.graph.pair_distances[0]) / f64::from(DISTANCE_SCALE) - expected)
                .abs()
                <= 0.00005
        );
    }

    #[test]
    fn each_exact_neighborhood_starts_with_self_and_has_finite_sorted_distances() {
        let profiles = vec![
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![0.5, 0.5],
        ];
        let embedding = build(&profiles);
        let graph = &embedding.graph;
        assert_eq!(
            graph.pair_distances.len(),
            profiles.len() * (profiles.len() - 1) / 2
        );
        for (index, (indices, distances)) in graph.indices.iter().zip(&graph.distances).enumerate()
        {
            assert_eq!(indices[0], index);
            assert_eq!(distances[0], 0.0);
            assert_eq!(indices.len(), 3);
            assert!(distances.windows(2).all(|pair| pair[0] <= pair[1]));
            assert!(distances.iter().all(|value| value.is_finite()));
        }
        assert!(build(&[]).features.is_empty());
    }

    #[test]
    fn global_distances_keep_muscle_affinity() {
        let profiles = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.8, 0.2, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let embedding = build(&profiles);
        let pairs = &embedding.graph.pair_distances;
        assert!(
            pairs[0] < pairs[1],
            "shared muscles connect different movements"
        );
        for (offset, (i, j)) in [(0, 1), (0, 2), (1, 2)].into_iter().enumerate() {
            let exact = distance(&embedding.features[i], &embedding.features[j]);
            assert!(
                (f64::from(pairs[offset]) / f64::from(DISTANCE_SCALE) - exact).abs() <= 0.00005
            );
        }
    }
}
