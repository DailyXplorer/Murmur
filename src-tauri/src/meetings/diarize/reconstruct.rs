//! From the windows and their clusters to the speaker segments of the track.
//!
//! Ported from `OfflineReconstruction.swift` (FluidAudio 0.17.7, revision
//! 503b4bd1): `buildSegments` (line 24), `mergeSegments` (line 431),
//! `sanitize` (line 481) and `excludeOverlaps` (line 359).
//!
//! The pass in short: every window votes, frame by frame, for the clusters of
//! its active local speakers. The frames of all windows are put on one global
//! grid (`frame_duration`, about 17 ms). On each grid frame the number of
//! speakers is the rounded mean of the windows' speaker counts, and the
//! clusters with the most votes fill those places. Runs of a cluster become
//! segments, near segments are merged, short ones dropped and overlaps cut so
//! that one speaker talks at a time.
//!
//! Not ported:
//! - the zero-vote re-embedding (`applyZeroVoteReembed`, lines 176-185 and
//!   249-295): the helper runs with `zeroVoteReembed = .disabled`
//!   (`OfflineDiarizerTypes.swift` line 279), so the Mac never runs it. Its
//!   effect is visible all the same: a frame where speech is counted but no
//!   window gave its speaker a cluster (assignment -2) has all-zero votes and
//!   the ranking breaks the tie towards cluster 0 (lines 167-174). That
//!   behaviour is kept.
//! - `buildSpeakerDatabase` (line 300) and the per-segment embedding and
//!   quality score: they do not change the times or speakers.
//! - the per-cluster vote counts and averages (lines 52-55, 106-143): they
//!   feed only the quality score.
//!
//! Where the Swift is not deterministic, this port fixes one order. Swift
//! `Dictionary` iteration order decides the order in which segments that end
//! on the same frame are appended (lines 196, 225), and so which of two
//! segments that start on the same frame is cut when they overlap. Here the
//! order is the cluster index, ascending.
//!
//! Times follow the Swift's precision on purpose: the grid is `f64`, segment
//! boundaries are rounded to `f32` when a segment is made (lines 424-425), and
//! the merge, the minimum duration and the overlap cut compare `f32` values
//! (lines 379, 483-489). Boundary cases therefore behave as on the Mac.
//!
//! Memory: the votes are a dense `frames x clusters` table of `f64`. A 3 hour
//! track has about 636,000 frames, so 5 MB per cluster (50 MB for 10
//! speakers), plus 12 bytes per frame for the speaker counts. Nothing else
//! grows with the track but the segments. The table is the only structure that
//! is not small; windows are read once, in order.

use super::config::{EXCLUSIVE_SEGMENTS, LOCAL_SPEAKERS, MIN_GAP_SECONDS, MIN_SEGMENT_SECONDS};
use super::types::{DiarSegment, Segmentation};

/// `Segmentation.community.minDurationOff`, `OfflineDiarizerTypes.swift` line
/// 50: with the gap setting it makes `gapThreshold` (Swift line 36).
const SEGMENTATION_MIN_DURATION_OFF: f64 = 0.0;
/// `Segmentation.community.minDurationOn`, line 49: with the minimum segment
/// duration it makes the minimum of `sanitize` (Swift line 483).
const SEGMENTATION_MIN_DURATION_ON: f64 = 0.0;
/// A guard on the vote table, in cells (`f64` each, 800 MB): about 10 times
/// what 3 hours with 30 speakers need. Input past it is a bug of the caller or
/// a timeline in the wrong unit, and gets no segments instead of an allocation
/// failure.
const MAX_VOTE_CELLS: usize = 100_000_000;

/// One run of a cluster on the grid, before merging.
#[derive(Debug, Clone, Copy)]
struct Raw {
    cluster: usize,
    start: f32,
    end: f32,
}

/// A run that is still growing: its first frame start and the end of its last
/// frame so far (`Accumulator`, Swift lines 8-13).
#[derive(Debug, Clone, Copy)]
struct Open {
    start: f64,
    end: f64,
}

/// Segments of the speakers of a track.
///
/// - `segmentation.chunks[i]` is window `i`; its `offset_seconds` places it.
/// - `hard_clusters[i][s]` is the cluster of local speaker `s` of window `i`;
///   a negative value (-1, -2: `buildChunkAssignments`,
///   `OfflineDiarizerManager.swift` line 974, fills -2) or an index past the
///   cluster count means the speaker has no cluster. A window without a row
///   has none either (Swift line 67).
/// - `centroids` only gives the number of clusters (`max(count, 1)`, Swift
///   line 35).
///
/// The result is ordered by start time and holds one speaker at a time
/// (`EXCLUSIVE_SEGMENTS`). `DiarSegment::speaker` is the cluster index. An
/// empty or degenerate input gives an empty result, never a panic.
pub fn build_segments(
    segmentation: &Segmentation,
    hard_clusters: &[Vec<i32>],
    centroids: &[Vec<f64>],
) -> Vec<DiarSegment> {
    if segmentation.chunks.is_empty() || segmentation.num_frames == 0 {
        return Vec::new();
    }
    let frame_duration: f64 = segmentation.frame_duration;
    if !frame_duration.is_finite() || frame_duration <= 0.0 {
        return Vec::new();
    }

    let cluster_count: usize = centroids.len().max(1);
    let gap_threshold: f64 = MIN_GAP_SECONDS.max(SEGMENTATION_MIN_DURATION_OFF);

    let mut max_time: f64 = 0.0;
    for chunk in &segmentation.chunks {
        let end: f64 = chunk.offset_seconds + segmentation.num_frames as f64 * frame_duration;
        if end > max_time {
            max_time = end;
        }
    }
    let frames_wanted: f64 = (max_time / frame_duration).ceil();
    if !frames_wanted.is_finite() || frames_wanted > MAX_VOTE_CELLS as f64 {
        log::warn!("reconstruction: timeline of {max_time} s is out of range, no segments");
        return Vec::new();
    }
    let total_frames: usize = (frames_wanted as usize).max(1);
    let cells: usize = match total_frames.checked_mul(cluster_count) {
        Some(cells) if cells <= MAX_VOTE_CELLS => cells,
        _ => {
            log::warn!("reconstruction: {total_frames} frames x {cluster_count} clusters is out of range, no segments");
            return Vec::new();
        }
    };

    let mut votes: Vec<f64> = vec![0.0; cells];
    let mut expected_sum: Vec<f64> = vec![0.0; total_frames];
    let mut expected_windows: Vec<u32> = vec![0; total_frames];

    accumulate_votes(
        segmentation,
        hard_clusters,
        cluster_count,
        &mut votes,
        &mut expected_sum,
        &mut expected_windows,
    );

    let raw: Vec<Raw> = runs_of_clusters(
        &votes,
        &expected_sum,
        &expected_windows,
        cluster_count,
        frame_duration,
    );

    let merged: Vec<Raw> = merge_segments(raw, gap_threshold);
    sanitize(merged)
}

/// Swift lines 59-104: put every frame of every window on the global grid and
/// add its votes. `votes[frame * clusters + cluster]` is the sum, over the
/// windows covering `frame`, of the largest weight among the local speakers of
/// that window mapped to `cluster`. `expected_sum` and `expected_windows` give
/// the mean speaker count (the sum of the weights of the frame).
fn accumulate_votes(
    segmentation: &Segmentation,
    hard_clusters: &[Vec<i32>],
    cluster_count: usize,
    votes: &mut [f64],
    expected_sum: &mut [f64],
    expected_windows: &mut [u32],
) {
    let frame_duration: f64 = segmentation.frame_duration;
    let total_frames: usize = expected_sum.len();
    let mut activation: Vec<f64> = vec![0.0; cluster_count];
    let mut touched: Vec<usize> = Vec::with_capacity(LOCAL_SPEAKERS);

    for (chunk_index, chunk) in segmentation.chunks.iter().enumerate() {
        let assignments: &[i32] = hard_clusters
            .get(chunk_index)
            .map(Vec::as_slice)
            .unwrap_or(&[]);

        for (frame_index, weights) in chunk.speaker_weights.iter().enumerate() {
            let frame_start: f64 = chunk.offset_seconds + frame_index as f64 * frame_duration;
            let rounded: f64 = (frame_start / frame_duration).round();
            let global: usize = if rounded.is_nan() || rounded < 0.0 {
                0
            } else if rounded >= total_frames as f64 {
                total_frames - 1
            } else {
                rounded as usize
            };

            touched.clear();
            for (speaker, &weight) in weights.iter().enumerate().take(assignments.len()) {
                let cluster: i32 = assignments[speaker];
                if cluster < 0 || cluster as usize >= cluster_count {
                    continue;
                }
                let cluster: usize = cluster as usize;
                let value: f64 = f64::from(weight);
                if value > activation[cluster] {
                    if activation[cluster] == 0.0 {
                        touched.push(cluster);
                    }
                    activation[cluster] = value;
                }
            }

            expected_sum[global] += weights.iter().fold(0.0, |sum, &w| sum + f64::from(w));
            expected_windows[global] = expected_windows[global].saturating_add(1);

            for &cluster in &touched {
                votes[global * cluster_count + cluster] += activation[cluster];
                activation[cluster] = 0.0;
            }
        }
    }
}

/// Swift lines 145-233: the speaker count of each grid frame, the clusters
/// that fill it, and the runs those clusters make. Returned in the order the
/// runs are closed (by end, then by cluster).
fn runs_of_clusters(
    votes: &[f64],
    expected_sum: &[f64],
    expected_windows: &[u32],
    cluster_count: usize,
    frame_duration: f64,
) -> Vec<Raw> {
    let total_frames: usize = expected_sum.len();
    let max_allowed: usize = cluster_count.min(LOCAL_SPEAKERS);
    let mut open: Vec<Option<Open>> = vec![None; cluster_count];
    let mut raw: Vec<Raw> = Vec::new();
    let mut selected: [usize; LOCAL_SPEAKERS] = [0; LOCAL_SPEAKERS];

    for frame in 0..total_frames {
        let frame_start: f64 = frame as f64 * frame_duration;
        let frame_end: f64 = frame_start + frame_duration;

        let windows: u32 = expected_windows[frame];
        let required: usize = if windows > 0 {
            // `.rounded(.toNearestOrEven)`, Swift line 151, then the clamp of
            // lines 152-153 (NaN becomes 0).
            let mean: f64 = (expected_sum[frame] / f64::from(windows)).round_ties_even();
            if mean.is_nan() {
                0
            } else {
                mean.clamp(0.0, max_allowed as f64) as usize
            }
        } else {
            0
        };

        let row: &[f64] = &votes[frame * cluster_count..(frame + 1) * cluster_count];
        select_top(row, required, &mut selected);
        let chosen: &[usize] = &selected[..required];

        for (cluster, slot) in open.iter_mut().enumerate() {
            if chosen.contains(&cluster) {
                continue;
            }
            if let Some(run) = slot.take() {
                push_run(&mut raw, cluster, run.start, frame_start);
            }
        }
        for &cluster in chosen {
            match open[cluster] {
                Some(ref mut run) => run.end = frame_end,
                None => {
                    open[cluster] = Some(Open {
                        start: frame_start,
                        end: frame_end,
                    })
                }
            }
        }
    }

    for (cluster, slot) in open.iter().enumerate() {
        if let Some(run) = *slot {
            push_run(&mut raw, cluster, run.start, run.end);
        }
    }
    raw
}

/// The `required` clusters with the most votes, most first; a tie goes to the
/// lower cluster index, as the stable descending sort of Swift line 171 does.
/// With no votes at all the first clusters win (the zero-vote case).
fn select_top(row: &[f64], required: usize, selected: &mut [usize; LOCAL_SPEAKERS]) {
    for place in 0..required {
        let mut best: Option<usize> = None;
        for (cluster, &value) in row.iter().enumerate() {
            if selected[..place].contains(&cluster) {
                continue;
            }
            let better: bool = match best {
                None => true,
                Some(current) => value > row[current],
            };
            if better {
                best = Some(cluster);
            }
        }
        match best {
            Some(cluster) => selected[place] = cluster,
            None => return,
        }
    }
}

/// `appendSegment`, Swift lines 400-429, without the embedding and quality:
/// nothing is made for an empty run, and the boundaries become `f32`.
fn push_run(output: &mut Vec<Raw>, cluster: usize, start: f64, end: f64) {
    if end > start {
        output.push(Raw {
            cluster,
            start: start as f32,
            end: end as f32,
        });
    }
}

/// Stable sort by start time, as Swift's `sorted` is in practice.
fn sort_by_start(segments: &mut [Raw]) {
    segments.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// `mergeSegments`, Swift lines 431-463. After sorting by start, a segment is
/// joined to the one before it when both are the same speaker and the gap is at
/// most `gap_threshold`. Only neighbours in the sorted list merge: a segment of
/// another speaker between two of one speaker keeps them apart, however small
/// the gap (lines 441-459).
fn merge_segments(mut segments: Vec<Raw>, gap_threshold: f64) -> Vec<Raw> {
    if segments.is_empty() {
        return segments;
    }
    sort_by_start(&mut segments);

    let mut merged: Vec<Raw> = Vec::with_capacity(segments.len());
    let mut current: Raw = segments[0];
    for &segment in &segments[1..] {
        if segment.cluster == current.cluster {
            let gap: f64 = f64::from(segment.start) - f64::from(current.end);
            if gap <= gap_threshold {
                current.end = current.end.max(segment.end);
                continue;
            }
        }
        merged.push(current);
        current = segment;
    }
    merged.push(current);
    merged
}

/// `sanitize`, Swift lines 481-496: sort, drop what is shorter than the
/// minimum, then make the segments exclusive.
fn sanitize(mut segments: Vec<Raw>) -> Vec<DiarSegment> {
    sort_by_start(&mut segments);
    let minimum: f32 = (MIN_SEGMENT_SECONDS as f32).max(SEGMENTATION_MIN_DURATION_ON as f32);
    segments.retain(|segment| segment.end - segment.start >= minimum);

    let kept: Vec<Raw> = if EXCLUSIVE_SEGMENTS {
        exclude_overlaps(&segments)
    } else {
        segments
    };
    kept.iter()
        .map(|segment| DiarSegment {
            speaker: segment.cluster,
            start: f64::from(segment.start),
            end: f64::from(segment.end),
        })
        .collect()
}

/// `excludeOverlaps`, Swift lines 359-398: a segment that starts before the
/// end of the one kept before it starts at that end instead; what is empty or
/// then shorter than the minimum duration is dropped. The previous segment is
/// the last one kept, not the last one seen.
fn exclude_overlaps(segments: &[Raw]) -> Vec<Raw> {
    let minimum: f32 = MIN_SEGMENT_SECONDS as f32;
    let mut kept: Vec<Raw> = Vec::with_capacity(segments.len());
    for segment in segments {
        let start: f32 = match kept.last() {
            Some(previous) if segment.start < previous.end => previous.end,
            _ => segment.start,
        };
        if start >= segment.end {
            continue;
        }
        if segment.end - start < minimum {
            continue;
        }
        kept.push(Raw {
            cluster: segment.cluster,
            start,
            end: segment.end,
        });
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::super::types::SegChunk;
    use super::*;

    /// A track grid in the tests: windows of 100 frames of 0.1 s, a window
    /// every 20 frames (2 s). Hand-checkable; reconstruction reads the grid
    /// from the `Segmentation`, not from the constants of the model.
    const FRAMES: usize = 100;
    const STEP_FRAMES: usize = 20;
    const FRAME_S: f64 = 0.1;

    /// A window at `index * step` whose local speakers are active on the given
    /// global frame ranges: `(local_speaker, from, to)`, `to` excluded.
    fn window(index: usize, active: &[(usize, usize, usize)]) -> SegChunk {
        window_on(index, STEP_FRAMES, FRAMES, FRAME_S, active)
    }

    fn window_on(
        index: usize,
        step_frames: usize,
        frames: usize,
        frame_s: f64,
        active: &[(usize, usize, usize)],
    ) -> SegChunk {
        let first: usize = index * step_frames;
        let mut weights: Vec<Vec<f32>> = vec![vec![0.0; 3]; frames];
        for &(speaker, from, to) in active {
            for global in from..to {
                if global >= first && global - first < frames {
                    weights[global - first][speaker] = 1.0;
                }
            }
        }
        SegChunk {
            index,
            offset_seconds: first as f64 * frame_s,
            frame_duration: frame_s,
            log_probs: Vec::new(),
            speaker_weights: weights,
        }
    }

    fn segmentation(chunks: Vec<SegChunk>) -> Segmentation {
        segmentation_on(chunks, FRAMES, FRAME_S)
    }

    fn segmentation_on(chunks: Vec<SegChunk>, frames: usize, frame_s: f64) -> Segmentation {
        Segmentation {
            chunks,
            num_frames: frames,
            frame_duration: frame_s,
        }
    }

    fn centroids(count: usize) -> Vec<Vec<f64>> {
        vec![vec![0.0; 4]; count]
    }

    /// (speaker, start, end) with times rounded to ms, for readable asserts.
    fn summary(segments: &[DiarSegment]) -> Vec<(usize, f64, f64)> {
        segments
            .iter()
            .map(|s| {
                (
                    s.speaker,
                    (s.start * 1000.0).round() / 1000.0,
                    (s.end * 1000.0).round() / 1000.0,
                )
            })
            .collect()
    }

    #[test]
    fn two_speakers_alternate_in_a_clean_window() {
        // Frame counts of 1, so each grid frame takes its one voted cluster
        // (Swift lines 145-174). Segments close on the frame where the
        // cluster stops (line 197).
        let seg = segmentation(vec![window(0, &[(0, 0, 30), (1, 30, 60)])]);
        let out = build_segments(&seg, &[vec![0, 1, -1]], &centroids(2));
        assert_eq!(summary(&out), vec![(0, 0.0, 3.0), (1, 3.0, 6.0)]);
    }

    #[test]
    fn overlapping_windows_vote_and_the_majority_wins() {
        // w0 -> cluster 0 on [0,60); w1 -> cluster 0 on [20,60); w2 -> cluster
        // 1 on [40,80). Frames 40..60 have votes 2 for cluster 0 and 1 for
        // cluster 1: cluster 0 (line 171). Frames 60..80 are active in w2 only:
        // the mean count of the three covering windows is 1/3, which rounds to
        // 0 (line 151), so nobody speaks there.
        let seg = segmentation(vec![
            window(0, &[(0, 0, 60)]),
            window(1, &[(0, 20, 60)]),
            window(2, &[(0, 40, 80)]),
        ]);
        let hard: Vec<Vec<i32>> = vec![vec![0, -2, -2], vec![0, -2, -2], vec![1, -2, -2]];
        let out = build_segments(&seg, &hard, &centroids(2));
        assert_eq!(summary(&out), vec![(0, 0.0, 6.0)]);
    }

    #[test]
    fn later_windows_can_outvote_earlier_ones_and_ties_go_to_the_lower_cluster() {
        // w0 -> 0 on [0,60); w1 -> 1 on [20,60); w2 -> 1 on [40,80).
        // [0,20): only w0 -> 0. [20,40): one vote each, tie -> cluster 0
        // (stable sort, line 171). [40,60): 1 vs 2 -> cluster 1.
        let seg = segmentation(vec![
            window(0, &[(0, 0, 60)]),
            window(1, &[(0, 20, 60)]),
            window(2, &[(0, 40, 80)]),
        ]);
        let hard: Vec<Vec<i32>> = vec![vec![0, -2, -2], vec![1, -2, -2], vec![1, -2, -2]];
        let out = build_segments(&seg, &hard, &centroids(2));
        assert_eq!(summary(&out), vec![(0, 0.0, 4.0), (1, 4.0, 6.0)]);
    }

    #[test]
    fn a_speaker_active_in_half_the_windows_rounds_to_silence() {
        // [20,60): w0 active, w1 silent: mean 0.5 rounds to 0, ties to even
        // (line 151). [0,20) is covered by w0 alone: mean 1.
        let seg = segmentation(vec![window(0, &[(0, 0, 60)]), window(1, &[])]);
        let out = build_segments(&seg, &[vec![0, -2, -2], vec![-2, -2, -2]], &centroids(1));
        assert_eq!(summary(&out), vec![(0, 0.0, 2.0)]);
    }

    #[test]
    fn overlap_of_two_speakers_is_cut_to_one_speaker_per_frame() {
        // Speaker 0 on [0,50), speaker 1 on [30,80): frames 30..50 count 2,
        // so both clusters are chosen there. Raw: c0 [0,5.0), c1 [3.0,8.0).
        // excludeOverlaps moves the start of c1 to 5.0 (lines 368-372).
        let seg = segmentation(vec![window(0, &[(0, 0, 50), (1, 30, 80)])]);
        let out = build_segments(&seg, &[vec![0, 1, -1]], &centroids(2));
        assert_eq!(summary(&out), vec![(0, 0.0, 5.0), (1, 5.0, 8.0)]);
        for pair in out.windows(2) {
            assert!(pair[0].end <= pair[1].start);
        }
    }

    #[test]
    fn a_segment_swallowed_by_the_previous_one_is_dropped() {
        // c0 [0,8.0) and c1 [3.0,5.0), both 1 s or more. c1 starts at the end
        // of c0 after the cut, 8.0 >= 5.0, so it disappears (line 374).
        let seg = segmentation(vec![window(0, &[(0, 0, 80), (1, 30, 50)])]);
        let out = build_segments(&seg, &[vec![0, 1, -1]], &centroids(2));
        assert_eq!(summary(&out), vec![(0, 0.0, 8.0)]);
    }

    #[test]
    fn the_cut_part_must_still_last_the_minimum_duration() {
        // c0 [0,5.0), c1 [3.0,5.5): after the cut c1 is [5.0,5.5), 0.5 s,
        // below the 1 s minimum (lines 378-381).
        let seg = segmentation(vec![window(0, &[(0, 0, 50), (1, 30, 55)])]);
        let out = build_segments(&seg, &[vec![0, 1, -1]], &centroids(2));
        assert_eq!(summary(&out), vec![(0, 0.0, 5.0)]);
    }

    #[test]
    fn short_gaps_of_one_speaker_merge_and_long_ones_stay() {
        // Windows of 1000 frames of 0.01 s. c0 on [0,200) and [205,400): a
        // gap of 0.05 s, below 0.1 s, merges (lines 443-444). The run on
        // [600,750) is 2 s later: a separate segment.
        let frames: usize = 1000;
        let frame_s: f64 = 0.01;
        let chunk = window_on(
            0,
            frames,
            frames,
            frame_s,
            &[(0, 0, 200), (0, 205, 400), (0, 600, 750)],
        );
        let seg = segmentation_on(vec![chunk], frames, frame_s);
        let out = build_segments(&seg, &[vec![0, -2, -2]], &centroids(1));
        assert_eq!(summary(&out), vec![(0, 0.0, 4.0), (0, 6.0, 7.5)]);
    }

    #[test]
    fn a_gap_of_more_than_the_threshold_is_kept() {
        let frames: usize = 1000;
        let frame_s: f64 = 0.01;
        let chunk = window_on(0, frames, frames, frame_s, &[(0, 0, 200), (0, 215, 400)]);
        let seg = segmentation_on(vec![chunk], frames, frame_s);
        let out = build_segments(&seg, &[vec![0, -2, -2]], &centroids(1));
        assert_eq!(summary(&out), vec![(0, 0.0, 2.0), (0, 2.15, 4.0)]);
    }

    #[test]
    fn another_speaker_between_two_runs_prevents_the_merge() {
        // Sorted by start, c0 [0,2.0) c1 [2.0,3.0) c0 [3.05,5.0): the two c0
        // runs are not neighbours, so the 0.05 s gap is not closed (lines
        // 437-459).
        let frames: usize = 1000;
        let frame_s: f64 = 0.01;
        let chunk = window_on(
            0,
            frames,
            frames,
            frame_s,
            &[(0, 0, 200), (1, 200, 300), (0, 305, 500)],
        );
        let seg = segmentation_on(vec![chunk], frames, frame_s);
        let out = build_segments(&seg, &[vec![0, 1, -2]], &centroids(2));
        assert_eq!(
            summary(&out),
            vec![(0, 0.0, 2.0), (1, 2.0, 3.0), (0, 3.05, 5.0)]
        );
    }

    #[test]
    fn segments_under_one_second_are_dropped() {
        // `sanitize` keeps duration >= max(minSegmentDuration, minDurationOn)
        // = 1 s (lines 483-489). 0.5 s goes, 1.5 s stays.
        let seg = segmentation(vec![window(0, &[(0, 0, 5), (1, 20, 35)])]);
        let out = build_segments(&seg, &[vec![0, 1, -1]], &centroids(2));
        assert_eq!(summary(&out), vec![(1, 2.0, 3.5)]);
    }

    #[test]
    fn a_speaker_without_cluster_in_a_silent_window_makes_no_segment() {
        let seg = segmentation(vec![window(0, &[])]);
        let out = build_segments(&seg, &[vec![-2, -2, -2]], &centroids(2));
        assert!(out.is_empty());
    }

    #[test]
    fn speech_without_any_cluster_falls_to_cluster_zero() {
        // The zero-vote case (lines 167-174, doc of 239-248): speech is
        // counted (mean 1) but no window gave the speaker a cluster, all
        // votes are 0 and the tie goes to cluster 0. The Mac does the same
        // with the re-embedding disabled.
        let seg = segmentation(vec![window(0, &[(0, 10, 40)])]);
        let out = build_segments(&seg, &[vec![-2, -2, -2]], &centroids(3));
        assert_eq!(summary(&out), vec![(0, 1.0, 4.0)]);

        // A missing row is the same as a row of -2 (line 67).
        let out = build_segments(&seg, &[], &centroids(3));
        assert_eq!(summary(&out), vec![(0, 1.0, 4.0)]);
    }

    #[test]
    fn the_speaker_count_cannot_exceed_the_cluster_count() {
        // Two local speakers active, both in cluster 0, one cluster in all:
        // maxAllowedSpeakers = min(1, 3) (line 147). One segment, not two.
        let seg = segmentation(vec![window(0, &[(0, 0, 40), (1, 0, 40)])]);
        let out = build_segments(&seg, &[vec![0, 0, -1]], &centroids(1));
        assert_eq!(summary(&out), vec![(0, 0.0, 4.0)]);
    }

    #[test]
    fn an_empty_centroid_list_still_has_cluster_zero() {
        // clusterCount = max(centroids.count, 1), line 35.
        let seg = segmentation(vec![window(0, &[(0, 0, 30)])]);
        let out = build_segments(&seg, &[vec![0, -1, -1]], &[]);
        assert_eq!(summary(&out), vec![(0, 0.0, 3.0)]);
    }

    #[test]
    fn a_cluster_past_the_count_counts_as_none() {
        // `cluster < clusterCount`, line 83. Cluster 5 of 2 is ignored, the
        // frame falls back to the zero-vote tie-break.
        let seg = segmentation(vec![window(0, &[(0, 0, 30)])]);
        let out = build_segments(&seg, &[vec![5, -1, -1]], &centroids(2));
        assert_eq!(summary(&out), vec![(0, 0.0, 3.0)]);
    }

    #[test]
    fn empty_and_degenerate_input_gives_nothing() {
        let empty = Segmentation::default();
        assert!(build_segments(&empty, &[], &[]).is_empty());

        let no_frames = Segmentation {
            chunks: vec![window(0, &[(0, 0, 30)])],
            num_frames: 0,
            frame_duration: FRAME_S,
        };
        assert!(build_segments(&no_frames, &[vec![0, 1, 2]], &centroids(3)).is_empty());

        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let seg = Segmentation {
                chunks: vec![window(0, &[(0, 0, 30)])],
                num_frames: FRAMES,
                frame_duration: bad,
            };
            assert!(build_segments(&seg, &[vec![0, 1, 2]], &centroids(3)).is_empty());
        }

        let mut silent = window(0, &[]);
        silent.speaker_weights.clear();
        let seg = segmentation(vec![silent]);
        assert!(build_segments(&seg, &[vec![0, 1, 2]], &centroids(3)).is_empty());
    }

    #[test]
    fn absurd_offsets_do_not_panic_or_allocate() {
        for offset in [f64::NAN, f64::INFINITY, -1.0e9, 1.0e15] {
            let mut chunk = window(0, &[(0, 0, 30)]);
            chunk.offset_seconds = offset;
            let seg = segmentation(vec![chunk]);
            let _ = build_segments(&seg, &[vec![0, 1, 2]], &centroids(3));
        }
    }

    #[test]
    fn the_same_input_gives_the_same_output() {
        // Several speakers, overlaps and equal start times; two runs equal.
        let seg = segmentation(vec![
            window(0, &[(0, 0, 50), (1, 30, 80)]),
            window(1, &[(0, 20, 60), (1, 40, 90)]),
            window(2, &[(0, 40, 70), (2, 50, 100)]),
            window(3, &[(2, 60, 120)]),
        ]);
        let hard: Vec<Vec<i32>> = vec![
            vec![0, 1, -2],
            vec![0, 1, -2],
            vec![1, -2, 2],
            vec![2, -2, -2],
        ];
        let first = build_segments(&seg, &hard, &centroids(3));
        let second = build_segments(&seg, &hard, &centroids(3));
        assert!(!first.is_empty());
        assert_eq!(first, second);
        for pair in first.windows(2) {
            assert!(pair[0].start <= pair[1].start);
            assert!(pair[0].end <= pair[1].start);
        }
    }

    #[test]
    fn real_window_geometry_places_windows_on_one_grid() {
        // 589 frames per 10 s window, a window every 2 s. A speaker talking
        // from 3 s to 9 s is seen by windows at 0, 2 and 4 s; the segment
        // lands within one frame (17 ms) of the truth.
        let frames: usize = 589;
        let frame_s: f64 = 10.0 / 589.0;
        let mut chunks: Vec<SegChunk> = Vec::new();
        for index in 0..6_usize {
            let offset: f64 = index as f64 * 2.0;
            let mut weights: Vec<Vec<f32>> = vec![vec![0.0; 3]; frames];
            for (f, row) in weights.iter_mut().enumerate() {
                let t: f64 = offset + f as f64 * frame_s;
                if (3.0..9.0).contains(&t) {
                    row[0] = 1.0;
                }
            }
            chunks.push(SegChunk {
                index,
                offset_seconds: offset,
                frame_duration: frame_s,
                log_probs: Vec::new(),
                speaker_weights: weights,
            });
        }
        let seg = segmentation_on(chunks, frames, frame_s);
        let hard: Vec<Vec<i32>> = vec![vec![0, -2, -2]; 6];
        let out = build_segments(&seg, &hard, &centroids(1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].speaker, 0);
        assert!((out[0].start - 3.0).abs() < 2.0 * frame_s, "{:?}", out[0]);
        assert!((out[0].end - 9.0).abs() < 2.0 * frame_s, "{:?}", out[0]);
    }
}
