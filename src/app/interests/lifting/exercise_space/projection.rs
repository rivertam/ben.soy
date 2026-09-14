//! Deterministic PCA for display only. Neighbors and training fit are computed
//! from the original muscle vectors, never from these projected coordinates.

pub(super) struct Projection {
    pub(super) positions: Vec<[f64; 3]>,
    pub(super) retained: f64,
}

pub(super) fn normalize(row: &[u32]) -> Vec<f64> {
    let norm = row
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    row.iter()
        .map(|value| {
            if norm > 0.0 {
                f64::from(*value) / norm
            } else {
                0.0
            }
        })
        .collect()
}

pub(super) fn similarity(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f64>()
        .clamp(0.0, 1.0)
}

// Jacobi rotations address symmetric matrix entries by their two coordinates.
#[allow(clippy::needless_range_loop)]
pub(super) fn project(rows: &[Vec<f64>]) -> Projection {
    let dimensions = rows.first().map_or(0, Vec::len);
    if rows.is_empty() || dimensions == 0 {
        return Projection {
            positions: vec![[0.0; 3]; rows.len()],
            retained: 0.0,
        };
    }
    let mut mean = vec![0.0; dimensions];
    for row in rows {
        for (mean, value) in mean.iter_mut().zip(row) {
            *mean += value / rows.len() as f64;
        }
    }
    let centered: Vec<Vec<f64>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .zip(&mean)
                .map(|(value, mean)| value - mean)
                .collect()
        })
        .collect();
    let mut covariance = vec![vec![0.0; dimensions]; dimensions];
    for row in &centered {
        for i in 0..dimensions {
            for j in 0..dimensions {
                covariance[i][j] += row[i] * row[j];
            }
        }
    }
    let total: f64 = (0..dimensions).map(|i| covariance[i][i]).sum();
    if total < 1e-12 {
        return Projection {
            positions: vec![[0.0; 3]; rows.len()],
            retained: 0.0,
        };
    }
    let mut basis = vec![vec![0.0; dimensions]; dimensions];
    for i in 0..dimensions {
        basis[i][i] = 1.0;
    }
    for _ in 0..dimensions * dimensions * 30 {
        let mut largest = 0.0;
        let (mut p, mut q) = (0, 0);
        for i in 0..dimensions {
            for j in i + 1..dimensions {
                if covariance[i][j].abs() > largest {
                    largest = covariance[i][j].abs();
                    p = i;
                    q = j;
                }
            }
        }
        if largest < total * 1e-12 {
            break;
        }
        let angle = 0.5 * (2.0 * covariance[p][q]).atan2(covariance[q][q] - covariance[p][p]);
        let (s, c) = angle.sin_cos();
        let (pp, qq, pq) = (covariance[p][p], covariance[q][q], covariance[p][q]);
        covariance[p][p] = c * c * pp - 2.0 * s * c * pq + s * s * qq;
        covariance[q][q] = s * s * pp + 2.0 * s * c * pq + c * c * qq;
        covariance[p][q] = 0.0;
        covariance[q][p] = 0.0;
        for i in 0..dimensions {
            if i != p && i != q {
                let (ip, iq) = (covariance[i][p], covariance[i][q]);
                covariance[i][p] = c * ip - s * iq;
                covariance[p][i] = covariance[i][p];
                covariance[i][q] = s * ip + c * iq;
                covariance[q][i] = covariance[i][q];
            }
            let (ip, iq) = (basis[i][p], basis[i][q]);
            basis[i][p] = c * ip - s * iq;
            basis[i][q] = s * ip + c * iq;
        }
    }
    let mut order: Vec<usize> = (0..dimensions).collect();
    order.sort_by(|left, right| {
        covariance[*right][*right]
            .total_cmp(&covariance[*left][*left])
            .then_with(|| left.cmp(right))
    });
    let mut axes = Vec::new();
    for index in order.iter().take(3) {
        let mut axis: Vec<f64> = basis.iter().map(|row| row[*index]).collect();
        // Fix arbitrary eigenvector signs for reproducible views.
        if axis
            .iter()
            .max_by(|left, right| left.abs().total_cmp(&right.abs()))
            .is_some_and(|value| *value < 0.0)
        {
            for value in &mut axis {
                *value = -*value;
            }
        }
        axes.push(axis);
    }
    Projection {
        positions: centered
            .iter()
            .map(|row| {
                let mut position = [0.0; 3];
                for (index, axis) in axes.iter().enumerate() {
                    position[index] = row.iter().zip(axis).map(|(value, axis)| value * axis).sum();
                }
                position
            })
            .collect(),
        retained: (order
            .iter()
            .take(3)
            .map(|index| covariance[*index][*index].max(0.0))
            .sum::<f64>()
            / total)
            .clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_three_projection_preserves_pairwise_distance() {
        let rows = vec![
            vec![1.0, 2.0, 3.0, 6.0],
            vec![2.0, 0.0, 1.0, 3.0],
            vec![0.0, 5.0, 2.0, 7.0],
            vec![3.0, 3.0, 3.0, 9.0],
        ];
        let projected = project(&rows);
        assert!((projected.retained - 1.0).abs() < 1e-9);
        for i in 0..rows.len() {
            for j in 0..rows.len() {
                let distance = |left: &[f64], right: &[f64]| {
                    left.iter()
                        .zip(right)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f64>()
                };
                assert!(
                    (distance(&rows[i], &rows[j])
                        - distance(&projected.positions[i], &projected.positions[j]))
                    .abs()
                        < 1e-8
                );
            }
        }
    }

    #[test]
    fn identical_and_empty_catalogs_are_finite_and_report_no_variation() {
        assert!(project(&[]).positions.is_empty());
        let projected = project(&[vec![1.0, 0.0], vec![1.0, 0.0]]);
        assert_eq!(projected.positions, [[0.0; 3]; 2]);
        assert_eq!(projected.retained, 0.0);
        assert_eq!(normalize(&[0, 0]), [0.0, 0.0]);
    }

    #[test]
    fn reducing_four_equal_variance_directions_reports_the_lost_quarter() {
        let rows: Vec<Vec<f64>> = (0..5)
            .map(|index| {
                (0..5)
                    .map(|other| if index == other { 1.0 } else { 0.0 })
                    .collect()
            })
            .collect();
        assert!((project(&rows).retained - 0.75).abs() < 1e-9);
    }

    #[test]
    fn similarity_uses_the_whole_profile_and_is_independent_of_magnitude() {
        assert!(
            (similarity(&normalize(&[100, 50, 0]), &normalize(&[50, 25, 0])) - 1.0).abs() < 1e-12
        );
        assert_eq!(
            similarity(&normalize(&[100, 0, 0]), &normalize(&[0, 0, 100])),
            0.0
        );
    }
}
