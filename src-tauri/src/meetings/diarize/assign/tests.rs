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
    let result = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
    assert_clean_partition(&scene, &result, 5);
}

#[test]
fn one_voice_over_many_windows_is_one_cluster() {
    let scene = scene(1, 200, 1, 0.0125, 3);
    let result = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
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

    let result = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
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
    let second = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
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
    let result = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
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
    let result = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
    assert_eq!(result.chunk_assignments.len(), 6);
}

#[test]
fn embeddings_outside_the_windows_are_ignored() {
    // Lines 986 to 994.
    let mut scene = scene(2, 20, 1, 0.01, 4);
    scene.embeddings[0].chunk_index = 500;
    scene.embeddings[1].speaker_index = 7;
    let result = cluster_embeddings(&scene.embeddings, &psi(), scene.windows).expect("clusters");
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
