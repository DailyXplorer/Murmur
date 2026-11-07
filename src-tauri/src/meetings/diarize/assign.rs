//! From the embeddings of a track to a speaker cluster for each local speaker of
//! each window: the clustering glue of `OfflineDiarizerManager.cluster`
//! (FluidAudio 0.17.7, revision 503b4bd1, `OfflineDiarizerManager.swift` lines
//! 335 to 423) and the helpers it calls, with the default configuration
//! (`Clustering.community`: no speaker count given, constrained assignment on).
//!
//! The steps: pick the training embeddings (the finite ones), cluster them with
//! `ahc` then `vbx`, take one centroid per cluster VBx kept, give EVERY
//! embedding the centroid that suits it best while two local speakers of one
//! window never share a centroid, and lay the result out as one row per window.
//!
//! Not ported, as the default configuration never reaches it: the
//! `SpeakerCountConstraints` branch (K-Means at a forced speaker count, lines
//! 378 to 392, and the `wasAdjusted` tests of `clusterAssignments`, lines 680 to
//! 683 and 713). Reconstruction, the zero-vote re-embedding and the exports
//! (lines 437 to 545) belong to other stages.
//!
//! `HungarianAssignment.maxScoreAssignment` is called by
//! `ConstrainedClusterAssignment.assign` (line 34) but is not among the spec
//! files. It is implemented here as the textbook optimal assignment (maximum
//! total score, every row to a different column, the rows with the lowest
//! scores left out when there are more rows than columns), which is what its
//! name and doc comment (lines 3 to 19) say. Which optimum it returns when two
//! assignments score exactly the same is not specified and not reproduced.

use std::borrow::Cow;
use std::collections::BTreeMap;

use super::ahc::{self, dot};
use super::config::{
    CLUSTERING_THRESHOLD, CONSTRAINED_ASSIGNMENT, EMBEDDING_DIM, LOCAL_SPEAKERS, PLDA_DIM,
};
use super::types::{StageError, TimedEmbedding};
use super::vbx::{self, VbxOutput, ACTIVE_CLUSTER_EPSILON};

/// What `buildChunkAssignments` puts where a local speaker has no cluster: it
/// had no embedding, or it was left out by the assignment (line 981;
/// `ConstrainedClusterAssignment.swift` line 25).
pub const UNASSIGNED: i32 = -2;

/// The result of the clustering stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Clustered {
    /// One centroid per cluster, 256 values each. The cluster index in
    /// `chunk_assignments` points here.
    pub centroids: Vec<Vec<f64>>,
    /// `num_chunks` rows of `LOCAL_SPEAKERS` entries: the cluster of each local
    /// speaker of each window, or `UNASSIGNED`.
    pub chunk_assignments: Vec<Vec<i32>>,
}

/// Clusters the embeddings of a track (`OfflineDiarizerManager.cluster`, lines
/// 335 to 423) with the `psi` of the PLDA model.
///
/// No embedding at all is `StageError::Invalid("no speech")`, the Swift
/// `noSpeechDetected` (line 348): the caller answers "no turns" for it. An
/// embedding of another size than the model's is also `Invalid`. Embeddings
/// with a non-finite value are left out of the training set but still get a
/// cluster (lines 632 to 652); if every one has such a value they all train.
///
/// Memory grows with the number of embeddings, about 6 KB each: the `f64`
/// copies of the 256 and 128 values, and one centroid each inside `ahc`.
pub fn cluster_embeddings(
    embeddings: &[TimedEmbedding],
    psi: &[f64],
    num_chunks: usize,
) -> Result<Clustered, StageError> {
    if embeddings.is_empty() {
        return Err(StageError::Invalid("no speech"));
    }
    if embeddings
        .iter()
        .any(|e| e.embedding256.len() != EMBEDDING_DIM || e.rho128.len() != PLDA_DIM)
    {
        return Err(StageError::Invalid("embedding size"));
    }

    let features: Vec<Vec<f64>> = embeddings
        .iter()
        .map(|e| e.embedding256.iter().map(|&x| f64::from(x)).collect())
        .collect();

    let training = select_training_embeddings(embeddings);
    let training_embeddings: Cow<[Vec<f64>]> = if training.len() == embeddings.len() {
        Cow::Borrowed(&features)
    } else {
        Cow::Owned(training.iter().map(|&i| features[i].clone()).collect())
    };
    let training_rho: Vec<Vec<f64>> = training
        .iter()
        .map(|&i| embeddings[i].rho128.clone())
        .collect();

    // Lines 366 to 374.
    let initial_clusters: Vec<usize> = if training_embeddings.len() >= 2 {
        ahc::cluster(&training_embeddings, CLUSTERING_THRESHOLD)
    } else {
        vec![0; training_embeddings.len()]
    };

    // Lines 376 to 398. `training` is never empty here, so the Swift branch
    // that skips VBx (lines 399 to 408) cannot happen.
    let vbx_output = vbx::refine(&training_rho, &initial_clusters, psi);

    let mut centroids = compute_centroids(&training_embeddings, &vbx_output, &initial_clusters);
    if centroids.is_empty() {
        centroids = compute_fallback_centroids(&features);
    }

    let chunk_indices: Vec<usize> = embeddings.iter().map(|e| e.chunk_index).collect();
    let assignments: Vec<i32> = if CONSTRAINED_ASSIGNMENT && centroids.len() > 1 {
        constrained_assign(&centroid_scores(&features, &centroids), &chunk_indices)
    } else {
        assign_embeddings(&features, &centroids)
    };

    let chunk_assignments =
        build_chunk_assignments(embeddings, &assignments, num_chunks, centroids.len());
    Ok(Clustered {
        centroids,
        chunk_assignments,
    })
}

/// `selectTrainingEmbeddings`, lines 632 to 652: the indices of the embeddings
/// with only finite values, or all of them when there is none.
fn select_training_embeddings(embeddings: &[TimedEmbedding]) -> Vec<usize> {
    let selected: Vec<usize> = embeddings
        .iter()
        .enumerate()
        .filter(|(_, e)| e.embedding256.iter().all(|x| x.is_finite()))
        .map(|(index, _)| index)
        .collect();
    if selected.is_empty() {
        (0..embeddings.len()).collect()
    } else {
        selected
    }
}

/// `computeCentroids`, lines 702 to 780, without the K-Means case: for every
/// cluster whose weight is above 1e-7, the mean of the training embeddings
/// weighted by their `gamma`; without VBx weights, the mean of each initial
/// cluster.
fn compute_centroids(
    training: &[Vec<f64>],
    vbx_output: &VbxOutput,
    initial_clusters: &[usize],
) -> Vec<Vec<f64>> {
    let Some(first) = training.first() else {
        return Vec::new();
    };
    let dimension = first.len();

    if !vbx_output.gamma.is_empty() && !vbx_output.pi.is_empty() {
        let active: Vec<usize> = vbx_output
            .pi
            .iter()
            .enumerate()
            .filter(|(_, weight)| **weight > ACTIVE_CLUSTER_EPSILON)
            .map(|(speaker, _)| speaker)
            .collect();
        if !active.is_empty() {
            let frame_limit = vbx_output.gamma.len().min(training.len());
            return active
                .iter()
                .map(|&speaker| {
                    let mut numerator: Vec<f64> = vec![0.0; dimension];
                    let mut denominator: f64 = 0.0;
                    for (row, embedding) in vbx_output.gamma.iter().zip(training).take(frame_limit)
                    {
                        let weight = row[speaker];
                        if weight > 0.0 {
                            denominator += weight;
                            for (sum, &x) in numerator.iter_mut().zip(embedding) {
                                *sum += weight * x;
                            }
                        }
                    }
                    if denominator > 0.0 {
                        numerator.iter().map(|sum| sum / denominator).collect()
                    } else {
                        vec![0.0; dimension]
                    }
                })
                .collect();
        }
    }

    compute_centroids_from_clusters(training, initial_clusters)
}

/// `computeCentroidsFromClusters`, lines 782 to 835: the mean of each cluster,
/// the clusters in order of label.
fn compute_centroids_from_clusters(embeddings: &[Vec<f64>], clusters: &[usize]) -> Vec<Vec<f64>> {
    if embeddings.is_empty() || embeddings.len() != clusters.len() {
        return Vec::new();
    }
    let mut grouped: BTreeMap<usize, (Vec<f64>, usize)> = BTreeMap::new();
    for (embedding, &cluster) in embeddings.iter().zip(clusters) {
        let entry = grouped
            .entry(cluster)
            .or_insert_with(|| (vec![0.0; embedding.len()], 0));
        for (sum, &x) in entry.0.iter_mut().zip(embedding) {
            *sum += x;
        }
        entry.1 += 1;
    }
    grouped
        .into_values()
        .map(|(sum, count)| sum.into_iter().map(|x| x / count as f64).collect())
        .collect()
}

/// `computeFallbackCentroids`, lines 837 to 875: the mean of everything.
fn compute_fallback_centroids(embeddings: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let Some(first) = embeddings.first() else {
        return Vec::new();
    };
    let mut accumulator: Vec<f64> = vec![0.0; first.len()];
    for vector in embeddings {
        for (sum, &x) in accumulator.iter_mut().zip(vector) {
            *sum += x;
        }
    }
    let scale = 1.0 / embeddings.len() as f64;
    vec![accumulator.into_iter().map(|x| x * scale).collect()]
}

/// `normalize`, lines 913 to 949: unit length, a zero vector stays as it is.
fn normalize(vector: &[f64]) -> Vec<f64> {
    let sum_squares = dot(vector, vector);
    if sum_squares <= 0.0 {
        return vector.to_vec();
    }
    let scale = 1.0 / sum_squares.sqrt();
    vector.iter().map(|x| x * scale).collect()
}

/// `centroidScores`, lines 878 to 887: the cosine of every embedding with every
/// centroid. An embedding with a non-finite value scores NaN everywhere.
fn centroid_scores(features: &[Vec<f64>], centroids: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let normalized_centroids: Vec<Vec<f64>> = centroids.iter().map(|c| normalize(c)).collect();
    features
        .iter()
        .map(|embedding| {
            let normalized = normalize(embedding);
            normalized_centroids
                .iter()
                .map(|centroid| {
                    if centroid.len() == normalized.len() {
                        dot(&normalized, centroid)
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect()
}

/// `assignEmbeddings`, lines 889 to 911: the centroid with the highest score,
/// the first one on a tie (and centroid 0 where no score beats -infinity).
fn assign_embeddings(features: &[Vec<f64>], centroids: &[Vec<f64>]) -> Vec<i32> {
    if features.is_empty() {
        return Vec::new();
    }
    if centroids.is_empty() {
        return vec![0; features.len()];
    }
    centroid_scores(features, centroids)
        .iter()
        .map(|row| {
            let mut best_index: usize = 0;
            let mut best_score = f64::NEG_INFINITY;
            for (index, &score) in row.iter().enumerate() {
                if score > best_score {
                    best_score = score;
                    best_index = index;
                }
            }
            best_index as i32
        })
        .collect()
}

/// `ConstrainedClusterAssignment.assign`, lines 20 to 42: within a window the
/// local speakers get different clusters, with the highest total score.
/// `UNASSIGNED` for a local speaker left out because the window has more of
/// them than there are clusters.
fn constrained_assign(scores: &[Vec<f64>], chunk_indices: &[usize]) -> Vec<i32> {
    let mut assignments: Vec<i32> = vec![UNASSIGNED; scores.len()];
    let mut rows_by_chunk: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (row, &chunk) in chunk_indices.iter().enumerate().take(scores.len()) {
        rows_by_chunk.entry(chunk).or_default().push(row);
    }
    for rows in rows_by_chunk.values() {
        let chunk_scores: Vec<&[f64]> = rows.iter().map(|&row| scores[row].as_slice()).collect();
        let assigned = max_score_assignment(&chunk_scores);
        for (&row, cluster) in rows.iter().zip(assigned) {
            if let Some(column) = cluster {
                assignments[row] = column as i32;
            }
        }
    }
    assignments
}

/// The assignment of rows to different columns that maximises the sum of the
/// scores (`HungarianAssignment.maxScoreAssignment`, see the module comment).
/// With more rows than columns, the rows left without a column are `None`. A
/// score that is not finite ranks below every finite one in the chunk.
fn max_score_assignment(scores: &[&[f64]]) -> Vec<Option<usize>> {
    let rows = scores.len();
    let columns = scores.iter().map(|row| row.len()).min().unwrap_or(0);
    if rows == 0 || columns == 0 {
        return vec![None; rows];
    }

    let lowest_finite = scores
        .iter()
        .flat_map(|row| row.iter().take(columns))
        .filter(|score| score.is_finite())
        .fold(f64::INFINITY, |lowest, &score| lowest.min(score));
    let floor = if lowest_finite.is_finite() {
        lowest_finite - 1.0
    } else {
        0.0
    };
    let gain = |row: usize, column: usize| -> f64 {
        let score = scores[row][column];
        if score.is_finite() {
            score
        } else {
            floor
        }
    };

    if rows <= columns {
        let cost: Vec<Vec<f64>> = (0..rows)
            .map(|r| (0..columns).map(|c| -gain(r, c)).collect())
            .collect();
        solve_assignment(&cost).into_iter().map(Some).collect()
    } else {
        // More rows than columns: assign every column to a row.
        let cost: Vec<Vec<f64>> = (0..columns)
            .map(|c| (0..rows).map(|r| -gain(r, c)).collect())
            .collect();
        let row_of_column = solve_assignment(&cost);
        let mut column_of_row: Vec<Option<usize>> = vec![None; rows];
        for (column, &row) in row_of_column.iter().enumerate() {
            column_of_row[row] = Some(column);
        }
        column_of_row
    }
}

/// Minimum-cost assignment of every row of `cost` (n rows, m >= n columns) to a
/// different column, by the shortest augmenting path method with potentials.
/// Returns the column of each row.
fn solve_assignment(cost: &[Vec<f64>]) -> Vec<usize> {
    let n = cost.len();
    let m = cost[0].len();
    // Index 0 of each array is the virtual column or row of the method.
    let mut row_potential: Vec<f64> = vec![0.0; n + 1];
    let mut column_potential: Vec<f64> = vec![0.0; m + 1];
    let mut row_of_column: Vec<usize> = vec![0; m + 1];
    let mut previous_column: Vec<usize> = vec![0; m + 1];

    for row in 1..=n {
        row_of_column[0] = row;
        let mut column: usize = 0;
        let mut min_to: Vec<f64> = vec![f64::INFINITY; m + 1];
        let mut used: Vec<bool> = vec![false; m + 1];
        loop {
            used[column] = true;
            let current_row = row_of_column[column];
            let mut delta = f64::INFINITY;
            let mut next_column: usize = 0;
            for candidate in 1..=m {
                if used[candidate] {
                    continue;
                }
                let reduced = cost[current_row - 1][candidate - 1]
                    - row_potential[current_row]
                    - column_potential[candidate];
                if reduced < min_to[candidate] {
                    min_to[candidate] = reduced;
                    previous_column[candidate] = column;
                }
                if min_to[candidate] < delta {
                    delta = min_to[candidate];
                    next_column = candidate;
                }
            }
            for candidate in 0..=m {
                if used[candidate] {
                    row_potential[row_of_column[candidate]] += delta;
                    column_potential[candidate] -= delta;
                } else {
                    min_to[candidate] -= delta;
                }
            }
            column = next_column;
            if row_of_column[column] == 0 {
                break;
            }
        }
        loop {
            let before = previous_column[column];
            row_of_column[column] = row_of_column[before];
            column = before;
            if column == 0 {
                break;
            }
        }
    }

    let mut column_of_row: Vec<usize> = vec![0; n];
    for column in 1..=m {
        if row_of_column[column] != 0 {
            column_of_row[row_of_column[column] - 1] = column - 1;
        }
    }
    column_of_row
}

/// `buildChunkAssignments`, lines 974 to 1000: `num_chunks` rows of
/// `LOCAL_SPEAKERS` entries, `UNASSIGNED` where a local speaker has no
/// embedding or no cluster. A later embedding for the same slot replaces an
/// earlier one.
fn build_chunk_assignments(
    embeddings: &[TimedEmbedding],
    assignments: &[i32],
    num_chunks: usize,
    cluster_count: usize,
) -> Vec<Vec<i32>> {
    let mut matrix: Vec<Vec<i32>> = vec![vec![UNASSIGNED; LOCAL_SPEAKERS]; num_chunks];
    for (embedding, &cluster) in embeddings.iter().zip(assignments) {
        if embedding.chunk_index >= matrix.len()
            || embedding.speaker_index >= LOCAL_SPEAKERS
            || cluster < 0
            || cluster as usize >= cluster_count
        {
            continue;
        }
        matrix[embedding.chunk_index][embedding.speaker_index] = cluster;
    }
    matrix
}

#[cfg(test)]
mod tests;
