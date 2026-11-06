//! Variational Bayes clustering (VBx), the second clustering stage. Ported from
//! `VBxClustering.swift` (FluidAudio 0.17.7, revision 503b4bd1), `refine`
//! (line 42) and `runVBx` (line 168), with the Accelerate calls written as plain
//! loops. It starts from the clusters of the agglomerative stage, runs EM on the
//! PLDA features and returns soft assignments (`gamma`) and the weight of each
//! cluster (`pi`): clusters VBx does not need end with a `pi` near 0.
//!
//! Not ported, because the default configuration never reaches it:
//! `refineWithConstraints` with `SpeakerCountConstraints` (lines 686 to 775,
//! K-Means at a forced speaker count). With no constraint it returns the
//! `refine` output unchanged (line 717), which is what `refine` here returns.
//! That also leaves out `centroids`, `wasAdjusted` and `originalClusterCount`
//! of `VBxOutput` (`OfflineDiarizerTypes.swift` lines 629 to 661), and the
//! training embeddings the K-Means needs.
//!
//! The only thing VBx reads from `PLDATransform` is `phiParameters`
//! (`PLDATransform.swift` line 22), the 128 values of `psi`: the caller passes
//! them as `psi`. The additions run in a fixed order, so one build gives one
//! answer, but the last digits can differ from the Accelerate sums, and `exp`
//! and `ln` come from the platform's math library.

use super::ahc::dot;
use super::config::{VBX_CONVERGENCE_TOLERANCE, VBX_FA, VBX_FB, VBX_MAX_ITERATIONS};

/// `initSmoothing: 7.0`, `VBxClustering.swift` line 132: the one-hot warm start
/// is softened to `softmax(7 * onehot)` before the first iteration.
const INIT_SMOOTHING: f64 = 7.0;
/// Floor of `psi` (line 240) and of the precision denominator (line 336).
const PHI_FLOOR: f64 = 1e-12;
/// Floor of `pi` before its logarithm (line 505).
const PI_FLOOR: f64 = 1e-8;
/// A cluster whose weight is above this is kept (`VBxOutput.activeClusterEpsilon`,
/// `OfflineDiarizerTypes.swift` line 664).
pub const ACTIVE_CLUSTER_EPSILON: f64 = 1e-7;

/// What `refine` returns (`VBxOutput`, `OfflineDiarizerTypes.swift` line 629).
#[derive(Debug, Clone, PartialEq)]
pub struct VbxOutput {
    /// One row per input, one column per warm-start cluster; each row sums to 1.
    pub gamma: Vec<Vec<f64>>,
    /// Weight of each warm-start cluster; sums to 1.
    pub pi: Vec<f64>,
    /// The `gamma` column with the highest value of each row, the first on a
    /// tie. The Swift wraps this row in one more array (line 158).
    pub hard_clusters: Vec<usize>,
    /// Warm-start cluster count: `max(1, distinct initial labels)` (line 79).
    pub num_clusters: usize,
    /// The ELBO after each iteration that ran.
    pub elbos: Vec<f64>,
}

impl VbxOutput {
    fn empty() -> VbxOutput {
        VbxOutput {
            gamma: Vec::new(),
            pi: Vec::new(),
            hard_clusters: Vec::new(),
            num_clusters: 0,
            elbos: Vec::new(),
        }
    }
}

/// VBx on the PLDA features `rho_features` (one row per embedding), warm
/// started from `initial_clusters` (one label per row, the agglomerative
/// result), with the `psi` of the PLDA model (`refine`, line 42). Uses `Fa`,
/// `Fb`, 20 iterations and a tolerance of 1e-4 from `config`.
///
/// No features, or rows of different lengths, give an empty output (the Swift
/// returns it for no features, line 46, and traps on jagged rows). A `psi` of
/// another length than the rows is replaced by ones (line 73).
pub fn refine(rho_features: &[Vec<f64>], initial_clusters: &[usize], psi: &[f64]) -> VbxOutput {
    let Some(first) = rho_features.first() else {
        return VbxOutput::empty();
    };
    let dimension = first.len();
    if dimension == 0 || rho_features.iter().any(|row| row.len() != dimension) {
        log::error!("VBx received empty or jagged feature vectors");
        return VbxOutput::empty();
    }
    let frame_count = rho_features.len();

    let phi: Vec<f64> = if psi.len() == dimension {
        psi.to_vec()
    } else {
        log::warn!(
            "PLDA psi dimension ({}) mismatches rho dimension ({dimension}); falling back to identity",
            psi.len()
        );
        vec![1.0; dimension]
    };

    let mut distinct: Vec<usize> = initial_clusters.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    let speaker_count = distinct.len().max(1);

    let features: Vec<f64> = rho_features.iter().flatten().copied().collect();

    let mut initial_gamma: Vec<f64> = vec![0.0; frame_count * speaker_count];
    if initial_clusters.is_empty() {
        initial_gamma.fill(1.0 / speaker_count as f64);
    } else {
        for (index, &cluster) in initial_clusters.iter().take(frame_count).enumerate() {
            initial_gamma[index * speaker_count + cluster.min(speaker_count - 1)] = 1.0;
        }
    }

    let result = run_vbx(&features, dimension, &phi, initial_gamma, speaker_count);

    let gamma: Vec<Vec<f64>> = result
        .gamma
        .chunks_exact(speaker_count)
        .map(<[f64]>::to_vec)
        .collect();
    let hard_clusters: Vec<usize> = gamma.iter().map(|row| first_maximum(row)).collect();

    VbxOutput {
        gamma,
        pi: result.pi,
        hard_clusters,
        num_clusters: speaker_count,
        elbos: result.elbos,
    }
}

/// Index of the highest value, the first one on a tie (Swift `max(by:)`).
fn first_maximum(row: &[f64]) -> usize {
    let mut best: usize = 0;
    for (index, &value) in row.iter().enumerate() {
        if value > row[best] {
            best = index;
        }
    }
    best
}

struct RunResult {
    gamma: Vec<f64>,
    pi: Vec<f64>,
    elbos: Vec<f64>,
}

/// Replaces `row` by `exp(row - max) / sum`, or by a uniform row when the sum
/// is not usable. Returns `(max, sum)` for the likelihood.
fn softmax_in_place(row: &mut [f64]) -> (f64, f64) {
    let mut row_max = -f64::MAX;
    for &value in row.iter() {
        if value > row_max {
            row_max = value;
        }
    }
    let mut sum_exp: f64 = 0.0;
    for value in row.iter_mut() {
        *value = (*value - row_max).exp();
        sum_exp += *value;
    }
    if sum_exp <= 0.0 || !sum_exp.is_finite() {
        let uniform = 1.0 / row.len() as f64;
        row.fill(uniform);
    } else {
        let inverse = 1.0 / sum_exp;
        for value in row.iter_mut() {
            *value *= inverse;
        }
    }
    (row_max, sum_exp)
}

/// `runVBx`, lines 168 to 665. `gamma` is `frame_count x speaker_count`, row
/// major, as in the Swift.
fn run_vbx(
    features: &[f64],
    dimension: usize,
    phi: &[f64],
    mut gamma: Vec<f64>,
    speaker_count: usize,
) -> RunResult {
    let fa = VBX_FA;
    let fb = VBX_FB;

    // Lines 191 to 220: soften the one-hot start.
    for row in gamma.chunks_exact_mut(speaker_count) {
        for value in row.iter_mut() {
            *value *= INIT_SMOOTHING;
        }
        softmax_in_place(row);
    }
    // Lines 222 to 236: make sure every row sums to 1.
    for row in gamma.chunks_exact_mut(speaker_count) {
        let sum: f64 = row.iter().sum();
        if sum <= 0.0 || !sum.is_finite() {
            row.fill(1.0 / speaker_count as f64);
        } else {
            let inverse = 1.0 / sum;
            for value in row.iter_mut() {
                *value *= inverse;
            }
        }
    }

    let mut pi: Vec<f64> = vec![1.0 / speaker_count as f64; speaker_count];

    let phi_clamped: Vec<f64> = phi.iter().map(|&p| p.max(PHI_FLOOR)).collect();
    let sqrt_phi: Vec<f64> = phi_clamped.iter().map(|p| p.sqrt()).collect();

    // Lines 243 to 266: rho = features * sqrt(phi), per column.
    let mut rho: Vec<f64> = Vec::with_capacity(features.len());
    for row in features.chunks_exact(dimension) {
        rho.extend(row.iter().zip(&sqrt_phi).map(|(x, s)| x * s));
    }

    // Lines 268 to 283.
    let log_constant = dimension as f64 * (2.0 * std::f64::consts::PI).ln();
    let g: Vec<f64> = features
        .chunks_exact(dimension)
        .map(|row| -0.5 * (row.iter().map(|x| x * x).sum::<f64>() + log_constant))
        .collect();

    let ratio = fa / fb;
    let table = speaker_count * dimension;
    let mut inv_l: Vec<f64> = vec![0.0; table];
    let mut alpha: Vec<f64> = vec![0.0; table];
    let mut temp: Vec<f64> = vec![0.0; table];
    let mut phi_offset: Vec<f64> = vec![0.0; speaker_count];
    let mut gamma_sum: Vec<f64> = vec![0.0; speaker_count];
    let mut scratch: Vec<f64> = vec![0.0; speaker_count];
    let mut elbos: Vec<f64> = Vec::with_capacity(VBX_MAX_ITERATIONS);

    let mut previous_elbo = -f64::MAX;

    for iteration in 0..VBX_MAX_ITERATIONS {
        // Lines 305 to 329: column sums of gamma.
        gamma_sum.fill(0.0);
        for row in gamma.chunks_exact(speaker_count) {
            for (sum, &value) in gamma_sum.iter_mut().zip(row) {
                *sum += value;
            }
        }

        // Lines 331 to 338.
        for (speaker, &sum) in gamma_sum.iter().enumerate() {
            let weight = ratio * sum;
            let row = &mut inv_l[speaker * dimension..(speaker + 1) * dimension];
            for (value, &p) in row.iter_mut().zip(&phi_clamped) {
                let denominator = 1.0 + weight * p;
                *value = 1.0 / denominator.max(PHI_FLOOR);
            }
        }

        // Lines 340 to 361: temp = gamma^T * rho. A zero weight adds nothing.
        temp.fill(0.0);
        for (gamma_row, rho_row) in gamma
            .chunks_exact(speaker_count)
            .zip(rho.chunks_exact(dimension))
        {
            for (speaker, &weight) in gamma_row.iter().enumerate() {
                if weight == 0.0 {
                    continue;
                }
                let target = &mut temp[speaker * dimension..(speaker + 1) * dimension];
                for (t, &r) in target.iter_mut().zip(rho_row) {
                    *t += weight * r;
                }
            }
        }

        // Lines 363 to 391: alpha = temp * invL * ratio.
        for ((a, &t), &l) in alpha.iter_mut().zip(&temp).zip(&inv_l) {
            *a = (t * l) * ratio;
        }

        // Lines 393 to 437 and 462 to 478: -0.5 * sum((alpha^2 + invL) * phi).
        for ((offset, alpha_row), inv_l_row) in phi_offset
            .iter_mut()
            .zip(alpha.chunks_exact(dimension))
            .zip(inv_l.chunks_exact(dimension))
        {
            let mut sum: f64 = 0.0;
            for ((&a, &l), &p) in alpha_row.iter().zip(inv_l_row).zip(&phi_clamped) {
                sum += (a * a + l) * p;
            }
            *offset = -0.5 * sum;
        }

        // Lines 497 to 517.
        let log_pi: Vec<f64> = pi.iter().map(|&p| p.max(PI_FLOOR).ln()).collect();

        // Lines 439 to 495 and 519 to 577: logP = (rho * alpha^T + offset + G) * Fa,
        // then the responsibilities of each row. Row `t` of gamma is read only
        // by the product above, so it can take the new values at once.
        let mut log_likelihood: f64 = 0.0;
        for ((gamma_row, rho_row), &g_t) in gamma
            .chunks_exact_mut(speaker_count)
            .zip(rho.chunks_exact(dimension))
            .zip(&g)
        {
            for (speaker, slot) in scratch.iter_mut().enumerate() {
                let alpha_row = &alpha[speaker * dimension..(speaker + 1) * dimension];
                let log_p = ((dot(rho_row, alpha_row) + phi_offset[speaker]) + g_t) * fa;
                *slot = log_p + log_pi[speaker];
            }
            let (row_max, sum_exp) = softmax_in_place(&mut scratch);
            gamma_row.copy_from_slice(&scratch);
            if sum_exp <= 0.0 || !sum_exp.is_finite() {
                log_likelihood += row_max;
            } else {
                log_likelihood += row_max + sum_exp.ln();
            }
        }

        // Lines 579 to 622: pi from the new gamma.
        pi.fill(0.0);
        for row in gamma.chunks_exact(speaker_count) {
            for (sum, &value) in pi.iter_mut().zip(row) {
                *sum += value;
            }
        }
        let pi_sum: f64 = pi.iter().sum();
        if pi_sum > 0.0 && pi_sum.is_finite() {
            let inverse = 1.0 / pi_sum;
            for value in pi.iter_mut() {
                *value *= inverse;
            }
        } else {
            pi.fill(1.0 / speaker_count as f64);
        }

        // Lines 624 to 648.
        let sum_log_inv: f64 = inv_l.iter().map(|v| v.ln()).sum();
        let sum_inv: f64 = inv_l.iter().sum();
        let sum_alpha_sq: f64 = alpha.iter().map(|a| a * a).sum();
        let elbo =
            log_likelihood + fb * 0.5 * (sum_log_inv - sum_inv - sum_alpha_sq + table as f64);

        elbos.push(elbo);

        // Lines 654 to 661: stop when the ELBO moved less than the tolerance.
        if iteration > 0 && (elbo - previous_elbo).abs() < VBX_CONVERGENCE_TOLERANCE {
            break;
        }
        previous_elbo = elbo;
    }

    RunResult { gamma, pi, elbos }
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

        /// Standard normal by Box-Muller.
        fn normal(&mut self) -> f64 {
            let u1 = self.next_f64().max(1e-300);
            let u2 = self.next_f64();
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        }
    }

    /// The PLDA generative model VBx assumes: a speaker mean with variance
    /// `psi` per dimension, plus unit noise per observation. `psi` falls from
    /// 25 to 0.5 like the shipped one, so it is a realistic scale.
    fn psi(dimension: usize) -> Vec<f64> {
        (0..dimension)
            .map(|d| 0.5 + 25.0 / (1.0 + d as f64))
            .collect()
    }

    fn blobs(
        speakers: usize,
        per_speaker: usize,
        dimension: usize,
        seed: u64,
    ) -> (Vec<Vec<f64>>, Vec<usize>) {
        let mut rng = Lcg(seed);
        let psi = psi(dimension);
        let means: Vec<Vec<f64>> = (0..speakers)
            .map(|_| psi.iter().map(|p| rng.normal() * p.sqrt()).collect())
            .collect();
        let mut rows: Vec<Vec<f64>> = Vec::new();
        let mut truth: Vec<usize> = Vec::new();
        for i in 0..speakers * per_speaker {
            let speaker = i % speakers;
            rows.push(means[speaker].iter().map(|m| m + rng.normal()).collect());
            truth.push(speaker);
        }
        (rows, truth)
    }

    #[test]
    fn empty_and_jagged_inputs_give_an_empty_output() {
        // Line 46.
        assert_eq!(refine(&[], &[], &psi(4)), VbxOutput::empty());
        assert_eq!(refine(&[vec![]], &[0], &[]), VbxOutput::empty());
        assert_eq!(
            refine(&[vec![1.0, 2.0], vec![1.0]], &[0, 0], &psi(2)),
            VbxOutput::empty()
        );
    }

    #[test]
    fn two_separated_blobs_keep_their_clusters() {
        let (rows, truth) = blobs(2, 60, 128, 5);
        let output = refine(&rows, &truth, &psi(128));

        assert_eq!(output.num_clusters, 2);
        // Every row of gamma is a distribution (lines 222 to 236, 529 to 572).
        for row in &output.gamma {
            assert_eq!(row.len(), 2);
            assert!((row.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        }
        // pi sums to 1 (lines 605 to 622) and both clusters keep a weight.
        assert!((output.pi.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(output.pi.iter().all(|&p| p > 0.4 && p < 0.6));
        // The hard assignment is the warm start.
        assert_eq!(output.hard_clusters, truth);
    }

    #[test]
    fn the_elbo_does_not_decrease_beyond_the_tolerance() {
        // EM never lowers its bound. Line 656 stops at a change below 1e-4, so
        // a drop larger than that would be a bug in the update order.
        let (rows, truth) = blobs(2, 80, 128, 9);
        let output = refine(&rows, &truth, &psi(128));
        assert!(!output.elbos.is_empty());
        assert!(output.elbos.len() <= VBX_MAX_ITERATIONS);
        for pair in output.elbos.windows(2) {
            assert!(
                pair[1] - pair[0] > -VBX_CONVERGENCE_TOLERANCE,
                "{:?}",
                output.elbos
            );
        }
        // It converged before the cap on such separated data, and the last two
        // values differ by less than the tolerance (line 656).
        let n = output.elbos.len();
        assert!(n >= 2);
        if n < VBX_MAX_ITERATIONS {
            assert!((output.elbos[n - 1] - output.elbos[n - 2]).abs() < VBX_CONVERGENCE_TOLERANCE);
        }
    }

    #[test]
    fn redundant_clusters_lose_their_weight() {
        // One speaker warm-started as three clusters, another as one: VBx has
        // to give the extra two clusters of the first speaker almost no weight
        // (`activeClusterCount`, `OfflineDiarizerTypes.swift` lines 675 to 678).
        let (rows, truth) = blobs(2, 90, 128, 21);
        let initial: Vec<usize> = rows
            .iter()
            .enumerate()
            .map(|(i, _)| if truth[i] == 0 { (i / 2) % 3 } else { 3 })
            .collect();
        let output = refine(&rows, &initial, &psi(128));
        assert_eq!(output.num_clusters, 4);
        for pair in output.elbos.windows(2) {
            assert!(
                pair[1] - pair[0] > -VBX_CONVERGENCE_TOLERANCE,
                "{:?}",
                output.elbos
            );
        }
        let active: usize = output
            .pi
            .iter()
            .filter(|&&p| p > ACTIVE_CLUSTER_EPSILON)
            .count();
        assert_eq!(active, 2, "pi {:?}", output.pi);
        // The two speakers end in different winning clusters, each one shared by
        // all the rows of its speaker.
        let winners_of = |speaker: usize| -> Vec<usize> {
            let mut seen: Vec<usize> = Vec::new();
            for (hard, &t) in output.hard_clusters.iter().zip(&truth) {
                if t == speaker && !seen.contains(hard) {
                    seen.push(*hard);
                }
            }
            seen
        };
        let first = winners_of(0);
        let second = winners_of(1);
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(second.len(), 1, "{second:?}");
        assert_ne!(first[0], second[0]);
    }

    #[test]
    fn a_single_row_is_one_cluster() {
        let output = refine(&[vec![0.5; 4]], &[0], &psi(4));
        assert_eq!(output.num_clusters, 1);
        assert_eq!(output.gamma, vec![vec![1.0]]);
        assert_eq!(output.pi, vec![1.0]);
        assert_eq!(output.hard_clusters, vec![0]);
    }

    #[test]
    fn a_psi_of_the_wrong_length_falls_back_to_ones() {
        // Line 73: same result as an explicit psi of ones.
        let (rows, truth) = blobs(2, 30, 16, 3);
        let wrong = refine(&rows, &truth, &psi(5));
        let ones = refine(&rows, &truth, &[1.0; 16]);
        assert_eq!(wrong, ones);
    }

    #[test]
    fn rows_after_the_initial_labels_start_uniform() {
        // Fewer labels than rows: the extra rows get zeros, which the
        // smoothing turns into a uniform row (lines 198 to 217). No panic.
        let (rows, truth) = blobs(2, 10, 8, 4);
        let output = refine(&rows, &truth[..10], &psi(8));
        assert_eq!(output.gamma.len(), rows.len());
    }

    #[test]
    fn same_input_gives_bit_identical_output() {
        let (rows, truth) = blobs(3, 40, 32, 8);
        assert_eq!(
            refine(&rows, &truth, &psi(32)),
            refine(&rows, &truth, &psi(32))
        );
    }

    #[test]
    fn non_finite_features_do_not_panic() {
        let rows: Vec<Vec<f64>> = vec![
            vec![f64::NAN, 1.0],
            vec![0.5, f64::INFINITY],
            vec![0.1, 0.2],
        ];
        let output = refine(&rows, &[0, 1, 0], &psi(2));
        assert_eq!(output.gamma.len(), 3);
        assert_eq!(output.pi.len(), 2);
    }

    /// Time of VBx on 16 000 rows warm started from 300 clusters. Run with
    /// `cargo test --release vbx_bench -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn vbx_bench() {
        for (rows_count, clusters) in [(1000usize, 30usize), (4000, 100), (16000, 300)] {
            let (rows, truth) = blobs(8, rows_count / 8, 128, 77);
            let initial: Vec<usize> = (0..rows.len())
                .map(|i| (truth[i] * clusters / 8) + (i / 8) % (clusters / 8))
                .collect();
            let start = std::time::Instant::now();
            let output = refine(&rows, &initial, &psi(128));
            let active = output
                .pi
                .iter()
                .filter(|&&p| p > ACTIVE_CLUSTER_EPSILON)
                .count();
            eprintln!(
                "vbx n={rows_count} start_clusters={clusters}: {:?}, {} iterations, {active} active, peak {} MB",
                start.elapsed(),
                output.elbos.len(),
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
