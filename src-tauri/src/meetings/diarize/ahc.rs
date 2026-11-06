//! Agglomerative clustering of the speaker embeddings, the first clustering
//! stage. Ported from `AHCClustering.swift` (FluidAudio 0.17.7, revision
//! 503b4bd1): L2-normalise, centroid linkage on the Euclidean distance, cut the
//! dendrogram at a distance, number the clusters in order of first appearance.
//!
//! The Swift calls `fastcluster_compute_centroid_linkage` (`AHCClustering.swift`
//! line 42), a C++ library that is not part of the spec files. Its contract is
//! the textbook one that `scipy.cluster.hierarchy.linkage(..., "centroid")`
//! also has: at each step merge the two clusters whose centroids are closest,
//! and record the Euclidean distance between those centroids. That is what this
//! module computes.
//!
//! Centroid linkage is not reducible: a merge can bring the new centroid closer
//! to a third cluster than the two parts were to each other (an inversion), so
//! the nearest-neighbour-chain shortcut gives other trees and cannot be used.
//! Instead of the N x N distance matrix (1 GB at 16 000 embeddings in `f64`)
//! this keeps one centroid per cluster and, per cluster, a lower bound on the
//! distance to the nearest cluster with a higher position, repaired lazily when
//! it reaches the top. Memory is N x dimension `f64` plus a few vectors of
//! length N; the tree it builds is the one the full matrix gives (checked
//! against a plain matrix implementation in the tests).

/// A merge of the dendrogram. Leaves are `0..count`; merge `k` creates node
/// `count + k` (the `fastcluster` numbering, `AHCClustering.swift` line 144).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Merge {
    left: usize,
    right: usize,
    distance: f64,
}

/// The cut distance range `clampDistanceThreshold` accepts
/// (`AHCClustering.swift` lines 112 to 121). Unit vectors are at most 2 apart.
const MAX_THRESHOLD: f64 = 2.0;

/// Labels of `embeddings` after agglomerative clustering with the centroid
/// linkage, cut at the Euclidean distance `threshold` (`AHCClustering.swift`
/// line 20). Labels count from 0 in the order the clusters first appear in
/// `embeddings` (`remapClusterIds`, line 200).
///
/// The result has one label per input row. An empty input gives an empty
/// result, a single row `[0]`, rows without values `0` each. Rows of different
/// lengths (a trap in the Swift) give one cluster per row, as the Swift does
/// when `fastcluster` fails (line 54). A non-finite value counts as 0, as the
/// Swift gives no defined result for it.
pub fn cluster(embeddings: &[Vec<f64>], threshold: f64) -> Vec<usize> {
    let count = embeddings.len();
    if count == 0 {
        return Vec::new();
    }
    let dimension = embeddings[0].len();
    if dimension == 0 {
        return vec![0; count];
    }
    if count == 1 {
        return vec![0];
    }
    if embeddings.iter().any(|row| row.len() != dimension) {
        return (0..count).collect();
    }

    let normalized = normalize_features(embeddings, dimension);
    let merges = centroid_linkage(normalized, count, dimension);
    let assignments = assignments_from_dendrogram(&merges, count, clamp_threshold(threshold));
    remap_cluster_ids(&assignments)
}

/// Rows scaled to unit length, in one buffer of `count x dimension`
/// (`normalizeFeatures`, lines 70 to 105). A row of norm 0 stays 0.
fn normalize_features(features: &[Vec<f64>], dimension: usize) -> Vec<f64> {
    let mut normalized: Vec<f64> = Vec::with_capacity(features.len() * dimension);
    for row in features {
        let start = normalized.len();
        normalized.extend(row.iter().map(|&x| if x.is_finite() { x } else { 0.0 }));
        let slice = &mut normalized[start..];
        let norm = dot(slice, slice);
        let scale = if norm > 0.0 { 1.0 / norm.sqrt() } else { 0.0 };
        for x in slice.iter_mut() {
            *x *= scale;
        }
    }
    normalized
}

/// `clampDistanceThreshold`, lines 112 to 121: NaN disables merging.
fn clamp_threshold(threshold: f64) -> f64 {
    if threshold.is_nan() {
        return 0.0;
    }
    threshold.clamp(0.0, MAX_THRESHOLD)
}

/// Dot product with four independent sums, so the compiler can vectorise it
/// without reordering the additions: the result does not depend on the target.
/// Shared with `vbx` and `assign`.
pub(super) fn dot(a: &[f64], b: &[f64]) -> f64 {
    let mut lanes: [f64; 4] = [0.0; 4];
    let (blocks_a, rest_a) = a.as_chunks::<4>();
    let (blocks_b, rest_b) = b.as_chunks::<4>();
    for (x, y) in blocks_a.iter().zip(blocks_b) {
        for ((lane, &p), &q) in lanes.iter_mut().zip(x).zip(y) {
            *lane += p * q;
        }
    }
    let mut tail: f64 = 0.0;
    for (&p, &q) in rest_a.iter().zip(rest_b) {
        tail += p * q;
    }
    (lanes[0] + lanes[1]) + (lanes[2] + lanes[3]) + tail
}

/// The clusters while they merge. `position` is a slot of `data`; a merge keeps
/// the cluster in the slot of the higher of its two parts.
struct Clusters {
    dimension: usize,
    /// Centroids, `count x dimension`; starts as the normalised points.
    data: Vec<f64>,
    /// `dot(centroid, centroid)` of each slot.
    norm2: Vec<f64>,
    /// Points in the cluster of each slot.
    size: Vec<f64>,
    active: Vec<bool>,
    /// Dendrogram node of each slot.
    node: Vec<usize>,
    /// Nearest active slot above this one, or `usize::MAX`.
    nearest: Vec<usize>,
    /// Squared distance to `nearest`, never above the true minimum.
    bound: Vec<f64>,
}

impl Clusters {
    fn new(data: Vec<f64>, count: usize, dimension: usize) -> Clusters {
        let norm2: Vec<f64> = (0..count)
            .map(|i| {
                let row = &data[i * dimension..(i + 1) * dimension];
                dot(row, row)
            })
            .collect();
        Clusters {
            dimension,
            data,
            norm2,
            size: vec![1.0; count],
            active: vec![true; count],
            node: (0..count).collect(),
            nearest: vec![usize::MAX; count],
            bound: vec![f64::INFINITY; count],
        }
    }

    fn row(&self, slot: usize) -> &[f64] {
        &self.data[slot * self.dimension..(slot + 1) * self.dimension]
    }

    /// Squared Euclidean distance between two centroids, from the norms and the
    /// dot product. Rounding can give a tiny negative value for equal points.
    fn distance2(&self, a: usize, b: usize) -> f64 {
        let value = self.norm2[a] + self.norm2[b] - 2.0 * dot(self.row(a), self.row(b));
        if value > 0.0 {
            value
        } else {
            0.0
        }
    }

    /// Sets `nearest` and `bound` of `slot` to the exact nearest active slot
    /// above it, the lowest one on a tie.
    fn rescan(&mut self, slot: usize) {
        let mut best: usize = usize::MAX;
        let mut best_distance: f64 = f64::INFINITY;
        for other in slot + 1..self.active.len() {
            if !self.active[other] {
                continue;
            }
            let distance = self.distance2(slot, other);
            if distance < best_distance {
                best_distance = distance;
                best = other;
            }
        }
        self.nearest[slot] = best;
        self.bound[slot] = best_distance;
    }

    /// The active slot with the smallest bound, the lowest one on a tie.
    fn smallest_bound(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        let mut best_bound: f64 = f64::INFINITY;
        for (slot, &bound) in self.bound.iter().enumerate() {
            if self.active[slot] && bound < best_bound {
                best_bound = bound;
                best = Some(slot);
            }
        }
        best
    }

    /// Merges `low` into `high`: the new centroid is the mean of the points,
    /// weighted by the sizes of the parts.
    fn merge_into(&mut self, low: usize, high: usize, node: usize) {
        let size_low = self.size[low];
        let size_high = self.size[high];
        let total = size_low + size_high;
        let dimension = self.dimension;
        let (head, tail) = self.data.split_at_mut(high * dimension);
        let from = &head[low * dimension..(low + 1) * dimension];
        let into = &mut tail[..dimension];
        for (target, &source) in into.iter_mut().zip(from) {
            *target = (size_low * source + size_high * *target) / total;
        }
        self.norm2[high] = dot(into, into);
        self.size[high] = total;
        self.active[low] = false;
        self.node[high] = node;
    }
}

/// The dendrogram of the centroid linkage: `count - 1` merges, each of the two
/// closest clusters at that point. Ties go to the lowest slots.
fn centroid_linkage(data: Vec<f64>, count: usize, dimension: usize) -> Vec<Merge> {
    let mut clusters = Clusters::new(data, count, dimension);
    for slot in 0..count - 1 {
        clusters.rescan(slot);
    }

    let mut merges: Vec<Merge> = Vec::with_capacity(count - 1);
    for step in 0..count - 1 {
        let (low, high, distance2) = loop {
            let Some(slot) = clusters.smallest_bound() else {
                break (usize::MAX, usize::MAX, f64::INFINITY);
            };
            let other = clusters.nearest[slot];
            if other != usize::MAX && clusters.active[other] {
                let distance2 = clusters.distance2(slot, other);
                if distance2 == clusters.bound[slot] {
                    break (slot, other, distance2);
                }
            }
            clusters.rescan(slot);
        };
        if low == usize::MAX {
            break;
        }

        merges.push(Merge {
            left: clusters.node[low],
            right: clusters.node[high],
            distance: distance2.sqrt(),
        });
        clusters.merge_into(low, high, count + step);

        for slot in 0..high {
            if !clusters.active[slot] {
                continue;
            }
            let distance = clusters.distance2(slot, high);
            if distance < clusters.bound[slot] {
                clusters.bound[slot] = distance;
                clusters.nearest[slot] = high;
            }
        }
        clusters.rescan(high);
    }
    merges
}

/// `assignmentsFromDendrogram`, lines 124 to 197. From the root down: a node at
/// or below the cut distance takes all its leaves as one cluster, whatever its
/// parts were (so an inversion, a parent closer than a child, is cut at the
/// parent); a node above it hands over to its two children.
fn assignments_from_dendrogram(merges: &[Merge], count: usize, threshold: f64) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![0];
    }
    if merges.len() + 1 != count {
        return (0..count).collect();
    }

    let mut assignments: Vec<Option<usize>> = vec![None; count];
    let mut stack: Vec<usize> = vec![count + merges.len() - 1];
    let mut next_label: usize = 0;

    while let Some(node) = stack.pop() {
        if node < count {
            if assignments[node].is_none() {
                assignments[node] = Some(next_label);
                next_label += 1;
            }
            continue;
        }
        let merge = merges[node - count];
        if merge.distance <= threshold {
            let label = next_label;
            next_label += 1;
            let mut queue: Vec<usize> = vec![node];
            while let Some(current) = queue.pop() {
                if current < count {
                    assignments[current] = Some(label);
                } else {
                    let part = merges[current - count];
                    queue.push(part.left);
                    queue.push(part.right);
                }
            }
        } else {
            stack.push(merge.left);
            stack.push(merge.right);
        }
    }

    assignments
        .into_iter()
        .map(|label| {
            label.unwrap_or_else(|| {
                let fresh = next_label;
                next_label += 1;
                fresh
            })
        })
        .collect()
}

/// `remapClusterIds`, lines 200 to 210: labels renumbered from 0 in order of
/// first appearance.
fn remap_cluster_ids(assignments: &[usize]) -> Vec<usize> {
    let mut seen: Vec<usize> = Vec::new();
    assignments
        .iter()
        .map(|&original| match seen.iter().position(|&s| s == original) {
            Some(index) => index,
            None => {
                seen.push(original);
                seen.len() - 1
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(angle: f64) -> Vec<f64> {
        vec![angle.cos(), angle.sin()]
    }

    /// Angle of two points on the unit circle that are `chord` apart.
    fn angle_for_chord(chord: f64) -> f64 {
        2.0 * (chord / 2.0).asin()
    }

    #[test]
    fn empty_single_and_valueless_inputs() {
        assert_eq!(cluster(&[], 0.6), Vec::<usize>::new());
        assert_eq!(cluster(&[vec![1.0, 0.0]], 0.6), vec![0]);
        // Lines 25 to 31: no dimension gives cluster 0 for each row.
        assert_eq!(cluster(&[vec![], vec![]], 0.6), vec![0, 0]);
    }

    #[test]
    fn jagged_rows_give_one_cluster_each_without_panicking() {
        assert_eq!(cluster(&[vec![1.0, 0.0], vec![1.0]], 0.6), vec![0, 1]);
    }

    #[test]
    fn identical_points_share_a_cluster() {
        // Distance 0 is under any cut (line 169).
        let labels = cluster(&[vec![0.3, 0.4], vec![0.3, 0.4]], 0.0);
        assert_eq!(labels, vec![0, 0]);
    }

    #[test]
    fn the_scale_of_a_row_does_not_matter() {
        // Features are scaled to unit length first (lines 70 to 105).
        let labels = cluster(&[vec![1.0, 0.0], vec![100.0, 0.0], vec![0.0, 5.0]], 0.6);
        assert_eq!(labels, vec![0, 0, 1]);
    }

    #[test]
    fn three_points_cut_at_each_distance() {
        // On the unit circle, A and B are 0.2 apart, C is 0.9 from A. The
        // centroid linkage merges A and B first, at 0.2. The mean of A and B
        // is then at distance `d` from C, which the test computes from the
        // coordinates (the definition of the linkage, not the code's formula).
        let a: f64 = 0.0;
        let b: f64 = angle_for_chord(0.2);
        let c: f64 = -angle_for_chord(0.9);
        let points: Vec<Vec<f64>> = vec![point(a), point(b), point(c)];
        let mid: Vec<f64> = vec![
            (points[0][0] + points[1][0]) / 2.0,
            (points[0][1] + points[1][1]) / 2.0,
        ];
        let second: f64 =
            ((mid[0] - points[2][0]).powi(2) + (mid[1] - points[2][1]).powi(2)).sqrt();
        assert!(second > 0.2);

        // Below the first merge: three clusters (line 169, `distance <= cut`).
        assert_eq!(cluster(&points, 0.19), vec![0, 1, 2]);
        // At the first merge distance and up to the second: {A,B}, {C}.
        assert_eq!(cluster(&points, 0.2 + 1e-9), vec![0, 0, 1]);
        assert_eq!(cluster(&points, second - 1e-9), vec![0, 0, 1]);
        // Above the root: one cluster.
        assert_eq!(cluster(&points, second + 1e-9), vec![0, 0, 0]);
    }

    #[test]
    fn labels_follow_first_appearance_not_tree_order() {
        // C comes first in the input and is far from the pair A, B.
        let points: Vec<Vec<f64>> = vec![point(3.0), point(0.0), point(0.05), point(3.02)];
        let labels = cluster(&points, 0.3);
        assert_eq!(labels, vec![0, 1, 1, 0]);
    }

    #[test]
    fn an_inversion_is_cut_at_the_parent() {
        // Three unit vectors 120 degrees apart are 3^0.5 = 1.732 from each
        // other. Any two merge at 1.732 and their mean, at (0.25, 0.433) for
        // 0 and 120 degrees, is 1.5 from the third: the root is closer than its
        // child. The Swift cut (lines 168 to 182) takes the whole tree as one
        // cluster at 1.6, where a "largest distance below" rule gives three.
        let points: Vec<Vec<f64>> = (0..3)
            .map(|k| point(k as f64 * 2.0 * std::f64::consts::PI / 3.0))
            .collect();
        assert_eq!(cluster(&points, 1.6), vec![0, 0, 0]);
        assert_eq!(cluster(&points, 1.4), vec![0, 1, 2]);
    }

    #[test]
    fn threshold_is_clamped_and_nan_disables_merging() {
        let points: Vec<Vec<f64>> = vec![point(0.0), point(0.1), point(3.0)];
        // Lines 113 to 120: NaN gives 0, above 2 gives 2 (everything merges).
        assert_eq!(cluster(&points, f64::NAN), vec![0, 1, 2]);
        assert_eq!(cluster(&points, 50.0), vec![0, 0, 0]);
        assert_eq!(cluster(&points, -1.0), vec![0, 1, 2]);
    }

    #[test]
    fn non_finite_values_do_not_panic() {
        let points: Vec<Vec<f64>> = vec![
            vec![f64::NAN, 1.0],
            vec![f64::INFINITY, 0.0],
            vec![0.0, 1.0],
        ];
        let labels = cluster(&points, 0.6);
        assert_eq!(labels.len(), 3);
    }

    struct Lcg(u64);

    impl Lcg {
        fn next_f64(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
        }
    }

    /// The textbook algorithm on the full distance matrix: merge the two
    /// closest centroids, update the matrix by recomputing from the centroids.
    fn reference_merges(points: &[Vec<f64>]) -> Vec<Merge> {
        let count = points.len();
        let mut centroids: Vec<Vec<f64>> = points.to_vec();
        for c in centroids.iter_mut() {
            let norm: f64 = c.iter().map(|x| x * x).sum::<f64>().sqrt();
            for x in c.iter_mut() {
                *x /= norm;
            }
        }
        let mut sizes: Vec<f64> = vec![1.0; count];
        let mut nodes: Vec<usize> = (0..count).collect();
        let mut alive: Vec<bool> = vec![true; count];
        let mut merges: Vec<Merge> = Vec::new();
        for step in 0..count - 1 {
            let mut best = (usize::MAX, usize::MAX);
            let mut best_distance = f64::INFINITY;
            for i in 0..count {
                for j in i + 1..count {
                    if !alive[i] || !alive[j] {
                        continue;
                    }
                    let d: f64 = centroids[i]
                        .iter()
                        .zip(&centroids[j])
                        .map(|(x, y)| (x - y) * (x - y))
                        .sum::<f64>()
                        .sqrt();
                    if d < best_distance {
                        best_distance = d;
                        best = (i, j);
                    }
                }
            }
            let (i, j) = best;
            merges.push(Merge {
                left: nodes[i],
                right: nodes[j],
                distance: best_distance,
            });
            let total = sizes[i] + sizes[j];
            let merged: Vec<f64> = centroids[i]
                .iter()
                .zip(&centroids[j])
                .map(|(x, y)| (sizes[i] * x + sizes[j] * y) / total)
                .collect();
            centroids[j] = merged;
            sizes[j] = total;
            nodes[j] = count + step;
            alive[i] = false;
        }
        merges
    }

    #[test]
    fn the_bounded_memory_algorithm_builds_the_matrix_algorithm_tree() {
        // Lines 42 to 50 ask for the centroid linkage; the reference is the
        // matrix version of its definition. Random data has no ties, so the
        // two trees must agree merge by merge, and so must every cut.
        let mut rng = Lcg(7);
        for (seed, count, dimension) in [
            (1u64, 5usize, 3usize),
            (2, 40, 4),
            (3, 120, 8),
            (4, 200, 2),
            (5, 150, 32),
        ] {
            rng.0 = seed;
            let points: Vec<Vec<f64>> = (0..count)
                .map(|_| (0..dimension).map(|_| rng.next_f64() - 0.5).collect())
                .collect();
            let normalized = normalize_features(&points, dimension);
            let fast = centroid_linkage(normalized, count, dimension);
            let slow = reference_merges(&points);
            assert_eq!(fast.len(), slow.len());
            for (a, b) in fast.iter().zip(&slow) {
                assert_eq!(
                    (a.left.min(a.right), a.left.max(a.right)),
                    (b.left.min(b.right), b.left.max(b.right)),
                    "seed {seed}"
                );
                assert!((a.distance - b.distance).abs() < 1e-9, "seed {seed}");
            }
            for threshold in [0.1, 0.3, 0.6, 0.9, 1.2] {
                let from_fast =
                    remap_cluster_ids(&assignments_from_dendrogram(&fast, count, threshold));
                let from_slow =
                    remap_cluster_ids(&assignments_from_dendrogram(&slow, count, threshold));
                assert_eq!(from_fast, from_slow, "seed {seed} threshold {threshold}");
            }
        }
    }

    #[test]
    fn same_input_gives_the_same_labels() {
        let mut rng = Lcg(11);
        let points: Vec<Vec<f64>> = (0..80)
            .map(|_| (0..6).map(|_| rng.next_f64() - 0.5).collect())
            .collect();
        assert_eq!(cluster(&points, 0.6), cluster(&points, 0.6));
    }

    /// Peak resident memory of the test process, from Linux `/proc` (0 elsewhere).
    fn peak_rss_mb() -> u64 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find(|line| line.starts_with("VmHWM:"))
                    .and_then(|line| line.split_whitespace().nth(1))
                    .and_then(|kb| kb.parse::<u64>().ok())
            })
            .map_or(0, |kb| kb / 1024)
    }

    /// Timing and memory of the linkage on 256-d data. Run with
    /// `cargo test --release ahc_bench -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ahc_bench() {
        let mut rng = Lcg(99);
        for (noise, count) in [(0.5, 4000usize), (0.6, 4000), (0.35, 16000), (0.5, 16000)] {
            let speakers: usize = 6;
            let dimension: usize = 256;
            let centers: Vec<Vec<f64>> = (0..speakers)
                .map(|_| (0..dimension).map(|_| rng.next_f64() - 0.5).collect())
                .collect();
            let points: Vec<Vec<f64>> = (0..count)
                .map(|i| {
                    centers[i % speakers]
                        .iter()
                        .map(|c| c + (rng.next_f64() - 0.5) * noise)
                        .collect()
                })
                .collect();
            let start = std::time::Instant::now();
            let labels = cluster(&points, 0.6);
            let clusters = labels.iter().max().map_or(0, |m| m + 1);
            eprintln!(
                "ahc noise={noise} n={count} dim={dimension}: {:?}, {clusters} clusters, peak {} MB",
                start.elapsed(),
                peak_rss_mb()
            );
        }
    }
}
