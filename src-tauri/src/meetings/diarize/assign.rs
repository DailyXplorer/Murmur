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
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn next_f64(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }

        fn normal(&mut self) -> f64 {
            let u1 = self.next_f64().max(1e-300);
            let u2 = self.next_f64();
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        }

        fn below(&mut self, bound: usize) -> usize {
            ((self.next_f64() * bound as f64) as usize).min(bound - 1)
        }
    }

    /// A PLDA `psi` shaped like the shipped one (25.9 down to 0.57).
    fn psi() -> Vec<f64> {
        (0..PLDA_DIM)
            .map(|d| 0.5 + 25.0 / (1.0 + d as f64))
            .collect()
    }

    struct Scene {
        embeddings: Vec<TimedEmbedding>,
        /// True speaker of each embedding.
        truth: Vec<usize>,
        windows: usize,
    }

    fn embedding(
        chunk: usize,
        slot: usize,
        direction: &[f64],
        mean: &[f64],
        noise: f64,
        rng: &mut Lcg,
    ) -> TimedEmbedding {
        let mut e256: Vec<f64> = direction.iter().map(|d| d + noise * rng.normal()).collect();
        let norm = e256.iter().map(|x| x * x).sum::<f64>().sqrt();
        for x in e256.iter_mut() {
            *x /= norm;
        }
        TimedEmbedding {
            chunk_index: chunk,
            speaker_index: slot,
            start_frame: 0,
            end_frame: 100,
            frame_weights: Vec::new(),
            start_time: chunk as f64 * 2.0,
            end_time: chunk as f64 * 2.0 + 4.0,
            embedding256: e256.iter().map(|&x| x as f32).collect(),
            rho128: mean.iter().map(|m| m + rng.normal()).collect(),
        }
    }

    /// `windows` windows over `speakers` voices. A window holds between 1 and
    /// `max_local` different voices, each one a local speaker. Directions are
    /// random unit vectors; the rho features follow the PLDA model (voice mean
    /// with variance `psi`, unit noise). `noise` is the per-coordinate noise of
    /// the 256-d embedding.
    fn scene(speakers: usize, windows: usize, max_local: usize, noise: f64, seed: u64) -> Scene {
        let mut rng = Lcg(seed);
        let psi = psi();
        let directions: Vec<Vec<f64>> = (0..speakers)
            .map(|_| {
                let v: Vec<f64> = (0..EMBEDDING_DIM).map(|_| rng.normal()).collect();
                let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
                v.iter().map(|x| x / norm).collect()
            })
            .collect();
        let means: Vec<Vec<f64>> = (0..speakers)
            .map(|_| psi.iter().map(|p| rng.normal() * p.sqrt()).collect())
            .collect();
        let mut embeddings: Vec<TimedEmbedding> = Vec::new();
        let mut truth: Vec<usize> = Vec::new();
        for window in 0..windows {
            let local = 1 + rng.below(max_local.min(speakers));
            let mut order: Vec<usize> = (0..speakers).collect();
            for i in (1..speakers).rev() {
                order.swap(i, rng.below(i + 1));
            }
            for (slot, &speaker) in order.iter().take(local).enumerate() {
                embeddings.push(embedding(
                    window,
                    slot,
                    &directions[speaker],
                    &means[speaker],
                    noise,
                    &mut rng,
                ));
                truth.push(speaker);
            }
        }
        Scene {
            embeddings,
            truth,
            windows,
        }
    }

    /// The clusters are the speakers: one cluster per voice, no two voices in a
    /// cluster, none left out.
    fn assert_clean_partition(scene: &Scene, result: &Clustered, speakers: usize) {
        assert_eq!(result.centroids.len(), speakers, "cluster count");
        assert_eq!(result.chunk_assignments.len(), scene.windows);
        let mut cluster_of_speaker: Vec<Option<i32>> = vec![None; speakers];
        for (e, &speaker) in scene.embeddings.iter().zip(&scene.truth) {
            let cluster = result.chunk_assignments[e.chunk_index][e.speaker_index];
            assert!(cluster >= 0, "embedding left out");
            match cluster_of_speaker[speaker] {
                None => cluster_of_speaker[speaker] = Some(cluster),
                Some(known) => assert_eq!(known, cluster, "voice {speaker} split over clusters"),
            }
        }
        let mut used: Vec<i32> = cluster_of_speaker
            .iter()
            .map(|c| c.expect("every voice appears"))
            .collect();
        used.sort_unstable();
        used.dedup();
        assert_eq!(used.len(), speakers, "two voices share a cluster");
    }

    #[test]
    fn two_three_five_and_eight_voices_are_recovered() {
        // Lines 366 to 423. Voices 5 and 8 are more than the 3 local speakers
        // of a window, so most windows hold only some of them.
        for (speakers, windows) in [(2usize, 150usize), (3, 150), (5, 250), (8, 400)] {
            let scene = scene(speakers, windows, 3, 0.0125, 100 + speakers as u64);
            let result =
                cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
            assert_clean_partition(&scene, &result, speakers);
        }
    }

    #[test]
    fn noisier_voices_that_the_first_stage_splits_are_still_recovered() {
        // Per-coordinate noise 0.04 is 0.64 in norm: two samples of a voice are
        // more than the 0.6 cut apart, so the agglomerative stage starts from
        // more clusters than voices.
        let scene = scene(5, 300, 3, 0.04, 77);
        let features: Vec<Vec<f64>> = scene
            .embeddings
            .iter()
            .map(|e| e.embedding256.iter().map(|&x| f64::from(x)).collect())
            .collect();
        let first_stage = ahc::cluster(&features, CLUSTERING_THRESHOLD);
        let first_stage_count = first_stage.iter().max().map_or(0, |m| m + 1);
        assert!(
            first_stage_count > 5,
            "the data must split the first stage: {first_stage_count}"
        );
        let result =
            cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        assert_clean_partition(&scene, &result, 5);
    }

    #[test]
    fn one_voice_over_many_windows_is_one_cluster() {
        let scene = scene(1, 200, 1, 0.0125, 3);
        let result =
            cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        assert_clean_partition(&scene, &result, 1);
    }

    #[test]
    fn two_local_speakers_of_one_voice_stay_together_with_a_single_centroid() {
        // Line 683: with one centroid the constraint is not applied, so two
        // local speakers that are the same voice both take cluster 0.
        let mut rng = Lcg(5);
        let direction: Vec<f64> = (0..EMBEDDING_DIM)
            .map(|i| if i == 0 { 1.0 } else { 0.0 })
            .collect();
        let mean: Vec<f64> = vec![1.0; PLDA_DIM];
        let embeddings: Vec<TimedEmbedding> = (0..10)
            .flat_map(|w| {
                let first = embedding(w, 0, &direction, &mean, 0.01, &mut rng);
                let second = embedding(w, 1, &direction, &mean, 0.01, &mut rng);
                [first, second]
            })
            .collect();
        let result = cluster_embeddings(&embeddings, &psi(), 10).expect("clusters");
        assert_eq!(result.centroids.len(), 1);
        for row in &result.chunk_assignments {
            assert_eq!(row[..2], [0, 0]);
            assert_eq!(row[2], UNASSIGNED);
        }
    }

    #[test]
    fn local_speakers_of_one_window_never_share_a_cluster() {
        // Three voices, 120 windows. Then one more window in which BOTH local
        // speakers sound like voice 0 (the second one a little less): a plain
        // nearest centroid would give both cluster 0.
        let mut scene = scene(3, 120, 3, 0.0125, 31);
        let mut rng = Lcg(999);
        let mut first_of_voice: Vec<Option<usize>> = vec![None; 3];
        for (index, &speaker) in scene.truth.iter().enumerate() {
            first_of_voice[speaker].get_or_insert(index);
        }
        let template = first_of_voice[0].expect("voice 0 appears");
        let window = scene.windows;
        for slot in 0..2 {
            let mut e = scene.embeddings[template].clone();
            e.chunk_index = window;
            e.speaker_index = slot;
            for x in e.embedding256.iter_mut() {
                *x += 0.005 * rng.normal() as f32;
            }
            scene.embeddings.push(e);
            scene.truth.push(0);
        }
        scene.windows += 1;

        let result =
            cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        assert_eq!(result.centroids.len(), 3);
        let row = &result.chunk_assignments[window];
        assert!(row[0] >= 0 && row[1] >= 0);
        assert_ne!(
            row[0], row[1],
            "the constrained assignment must separate them"
        );

        // Without the constraint, both would snap to the same centroid.
        let features: Vec<Vec<f64>> = scene
            .embeddings
            .iter()
            .map(|e| e.embedding256.iter().map(|&x| f64::from(x)).collect())
            .collect();
        let plain = assign_embeddings(&features, &result.centroids);
        let last = scene.embeddings.len();
        assert_eq!(
            plain[last - 1],
            plain[last - 2],
            "the test must need the constraint"
        );

        for row in &result.chunk_assignments {
            let mut seen: Vec<i32> = row.iter().copied().filter(|&c| c >= 0).collect();
            let before = seen.len();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(
                seen.len(),
                before,
                "a window uses one cluster twice: {row:?}"
            );
        }
    }

    #[test]
    fn same_input_gives_the_same_result_twice() {
        let scene = scene(4, 120, 3, 0.02, 12);
        let first = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        let second =
            cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        assert_eq!(first, second);
    }

    #[test]
    fn no_embeddings_is_no_speech() {
        // Line 348: `noSpeechDetected`.
        assert_eq!(
            cluster_embeddings(&[], &psi(), 5),
            Err(StageError::Invalid("no speech"))
        );
    }

    #[test]
    fn an_embedding_of_the_wrong_size_is_invalid() {
        let mut scene = scene(2, 4, 1, 0.01, 1);
        scene.embeddings[0].embedding256.pop();
        assert_eq!(
            cluster_embeddings(&scene.embeddings, &psi(), 4),
            Err(StageError::Invalid("embedding size"))
        );
    }

    #[test]
    fn a_single_embedding_is_one_cluster() {
        let scene = scene(1, 1, 1, 0.01, 2);
        let result = cluster_embeddings(&scene.embeddings, &psi(), 1).expect("clusters");
        assert_eq!(result.centroids.len(), 1);
        assert_eq!(
            result.chunk_assignments,
            vec![vec![0, UNASSIGNED, UNASSIGNED]]
        );
    }

    #[test]
    fn two_identical_embeddings_are_one_cluster() {
        let scene = scene(1, 1, 1, 0.01, 2);
        let mut twin = scene.embeddings[0].clone();
        twin.chunk_index = 1;
        let both = vec![scene.embeddings[0].clone(), twin];
        let result = cluster_embeddings(&both, &psi(), 2).expect("clusters");
        assert_eq!(result.centroids.len(), 1);
        assert_eq!(result.chunk_assignments[0][0], 0);
        assert_eq!(result.chunk_assignments[1][0], 0);
    }

    #[test]
    fn nan_embeddings_are_left_out_of_training_and_do_not_panic() {
        let mut scene = scene(3, 90, 2, 0.0125, 8);
        let clean = scene.embeddings.len();
        let mut broken = scene.embeddings[0].clone();
        broken.chunk_index = scene.windows;
        broken.speaker_index = 0;
        broken.embedding256[3] = f32::NAN;
        scene.embeddings.push(broken);
        scene.windows += 1;

        assert_eq!(select_training_embeddings(&scene.embeddings).len(), clean);
        let result =
            cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        // The clean embeddings are clustered exactly as without the broken one.
        assert_eq!(result.centroids.len(), 3);
        let truth_clean = Scene {
            embeddings: scene.embeddings[..clean].to_vec(),
            truth: scene.truth.clone(),
            windows: scene.windows - 1,
        };
        let mut trimmed = result.clone();
        trimmed.chunk_assignments.pop();
        assert_clean_partition(&truth_clean, &trimmed, 3);
    }

    #[test]
    fn only_nan_embeddings_still_answer() {
        // Lines 647 to 649: with no finite embedding, all of them train.
        let mut scene = scene(2, 6, 1, 0.01, 6);
        for e in scene.embeddings.iter_mut() {
            e.embedding256[0] = f32::NAN;
        }
        assert_eq!(
            select_training_embeddings(&scene.embeddings).len(),
            scene.embeddings.len()
        );
        let result =
            cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        assert_eq!(result.chunk_assignments.len(), 6);
    }

    #[test]
    fn embeddings_outside_the_windows_are_ignored() {
        // Lines 986 to 994.
        let mut scene = scene(2, 20, 1, 0.01, 4);
        scene.embeddings[0].chunk_index = 500;
        scene.embeddings[1].speaker_index = 7;
        let result =
            cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
        assert_eq!(result.chunk_assignments.len(), 20);
        assert!(result
            .chunk_assignments
            .iter()
            .all(|row| row.len() == LOCAL_SPEAKERS));
        assert_eq!(result.chunk_assignments[1][0], UNASSIGNED);
    }

    #[test]
    fn no_windows_gives_no_rows() {
        let scene = scene(2, 6, 1, 0.01, 4);
        let result = cluster_embeddings(&scene.embeddings, &psi(), 0).expect("clusters");
        assert!(result.chunk_assignments.is_empty());
    }

    fn chunk_rows(rows: &[&[f64]]) -> Vec<Vec<f64>> {
        rows.iter().map(|r| r.to_vec()).collect()
    }

    #[test]
    fn constrained_assignment_of_two_by_two() {
        // Both rows prefer column 0, 0.9 and 0.95. Giving column 0 to row 1 and
        // column 1 to row 0 totals 0.8 + 0.95 = 1.75, against 0.9 + 0.1 = 1.0.
        let scores = chunk_rows(&[&[0.9, 0.8], &[0.95, 0.1]]);
        assert_eq!(constrained_assign(&scores, &[0, 0]), vec![1, 0]);
    }

    #[test]
    fn constrained_assignment_of_three_by_three() {
        // Rows 0 and 1 both prefer column 0. The permutation [1, 0, 2] totals
        // 0.85 + 0.95 + 0.9 = 2.7; the next best, [0, 1, 2], 0.9 + 0.2 + 0.9 = 2.0.
        let scores = chunk_rows(&[&[0.9, 0.85, 0.1], &[0.95, 0.2, 0.1], &[0.3, 0.4, 0.9]]);
        assert_eq!(constrained_assign(&scores, &[4, 4, 4]), vec![1, 0, 2]);
    }

    #[test]
    fn more_local_speakers_than_clusters_leave_the_weakest_out() {
        // Three rows, two columns: rows 0 and 2 with columns 0 and 1 total
        // 0.9 + 0.7 = 1.6, the best of the three pairs (1.5 and 1.1 for the
        // others). Row 1 is dropped: -2 (`ConstrainedClusterAssignment.swift`
        // line 17).
        let scores = chunk_rows(&[&[0.9, 0.1], &[0.8, 0.2], &[0.1, 0.7]]);
        assert_eq!(
            constrained_assign(&scores, &[0, 0, 0]),
            vec![0, UNASSIGNED, 1]
        );
    }

    #[test]
    fn chunks_are_assigned_independently_whatever_their_order() {
        // Rows of chunk 9 and chunk 2 are interleaved; each chunk solves alone
        // (lines 27 to 39), so chunk 9 may reuse column 0.
        let scores = chunk_rows(&[&[0.9, 0.1], &[0.9, 0.1], &[0.2, 0.8], &[0.7, 0.6]]);
        assert_eq!(constrained_assign(&scores, &[9, 2, 9, 2]), vec![0, 0, 1, 1]);
    }

    #[test]
    fn non_finite_scores_rank_below_finite_ones() {
        // Line 16: the NaN row takes what is left; the finite row keeps its best.
        let scores = chunk_rows(&[&[f64::NAN, f64::NAN], &[0.2, 0.9]]);
        assert_eq!(constrained_assign(&scores, &[0, 0]), vec![0, 1]);
        // One column, two rows: the finite row wins it.
        let scores = chunk_rows(&[&[f64::NAN], &[0.5]]);
        assert_eq!(constrained_assign(&scores, &[0, 0]), vec![UNASSIGNED, 0]);
    }

    #[test]
    fn no_rows_no_columns_no_panic() {
        assert_eq!(constrained_assign(&[], &[]), Vec::<i32>::new());
        assert_eq!(constrained_assign(&[vec![]], &[0]), vec![UNASSIGNED]);
    }

    #[test]
    fn the_optimal_assignment_matches_brute_force_on_random_matrices() {
        let mut rng = Lcg(2024);
        for case in 0..200 {
            let rows = 1 + rng.below(4);
            let columns = 1 + rng.below(5);
            let scores: Vec<Vec<f64>> = (0..rows)
                .map(|_| (0..columns).map(|_| rng.next_f64()).collect())
                .collect();
            let views: Vec<&[f64]> = scores.iter().map(Vec::as_slice).collect();
            let found = max_score_assignment(&views);
            let found_total: f64 = found
                .iter()
                .enumerate()
                .filter_map(|(r, c)| c.map(|c| scores[r][c]))
                .sum();
            let assigned = found.iter().flatten().count();
            assert_eq!(assigned, rows.min(columns), "case {case}");
            let mut columns_used: Vec<usize> = found.iter().flatten().copied().collect();
            columns_used.sort_unstable();
            columns_used.dedup();
            assert_eq!(
                columns_used.len(),
                assigned,
                "case {case}: a column used twice"
            );

            fn best(scores: &[Vec<f64>], row: usize, used: &mut [bool], skipped: usize) -> f64 {
                if row == scores.len() {
                    return 0.0;
                }
                let mut top = f64::NEG_INFINITY;
                if skipped > 0 {
                    top = top.max(best(scores, row + 1, used, skipped - 1));
                }
                for c in 0..used.len() {
                    if !used[c] {
                        used[c] = true;
                        top = top.max(scores[row][c] + best(scores, row + 1, used, skipped));
                        used[c] = false;
                    }
                }
                top
            }
            let skipped = rows.saturating_sub(columns);
            let optimum = best(&scores, 0, &mut vec![false; columns], skipped);
            assert!(
                (found_total - optimum).abs() < 1e-9,
                "case {case}: {found_total} vs {optimum}"
            );
        }
    }

    #[test]
    fn centroids_are_the_gamma_weighted_means() {
        // Lines 724 to 772 on a hand-made output: two clusters, gamma 1/0 and
        // 0/1 for the first two rows, 0.5/0.5 for the third.
        let training = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![1.0, 1.0]];
        let output = VbxOutput {
            gamma: vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.5, 0.5]],
            pi: vec![0.5, 0.5],
            hard_clusters: vec![0, 1, 0],
            num_clusters: 2,
            elbos: Vec::new(),
        };
        let centroids = compute_centroids(&training, &output, &[0, 1, 0]);
        // Cluster 0: (1*[1,0] + 0.5*[1,1]) / 1.5 ; cluster 1: (1*[0,1] + 0.5*[1,1]) / 1.5.
        let expected = [[1.5 / 1.5, 0.5 / 1.5], [0.5 / 1.5, 1.5 / 1.5]];
        for (centroid, want) in centroids.iter().zip(expected) {
            for (a, b) in centroid.iter().zip(want) {
                assert!((a - b).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn a_cluster_with_no_weight_is_not_a_centroid_and_empty_output_falls_back() {
        let training = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let output = VbxOutput {
            gamma: vec![vec![1.0, 0.0], vec![1.0, 0.0]],
            pi: vec![1.0, 0.0],
            hard_clusters: vec![0, 0],
            num_clusters: 2,
            elbos: Vec::new(),
        };
        // Line 725: pi below 1e-7 is not active.
        assert_eq!(compute_centroids(&training, &output, &[0, 1]).len(), 1);
        // No VBx output: the means of the initial clusters, in label order (line 820).
        let empty = VbxOutput {
            gamma: Vec::new(),
            pi: Vec::new(),
            hard_clusters: Vec::new(),
            num_clusters: 0,
            elbos: Vec::new(),
        };
        assert_eq!(
            compute_centroids(&training, &empty, &[1, 0]),
            vec![vec![0.0, 1.0], vec![1.0, 0.0]]
        );
    }

    /// Time and size of the whole stage on 16 000 embeddings. Run with
    /// `cargo test --release stage_bench -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn stage_bench() {
        for windows in [400usize, 1600, 6400] {
            let scene = scene(6, windows, 3, 0.03, 4242);
            let start = std::time::Instant::now();
            let result =
                cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
            eprintln!(
                "stage windows={windows} embeddings={}: {:?}, {} centroids, peak {} MB",
                scene.embeddings.len(),
                start.elapsed(),
                result.centroids.len(),
                std::fs::read_to_string("/proc/self/status")
                    .ok()
                    .and_then(|status| status
                        .lines()
                        .find(|line| line.starts_with("VmHWM:"))
                        .and_then(|line| line.split_whitespace().nth(1))
                        .and_then(|kb| kb.parse::<u64>().ok()))
                    .map_or(0, |kb| kb / 1024)
            );
        }
    }
}
