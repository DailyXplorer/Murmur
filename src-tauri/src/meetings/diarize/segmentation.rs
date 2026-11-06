//! Windows of the segmentation model and the decoding of its output.
//!
//! Ported from `OfflineSegmentationProcessor.swift` (FluidAudio 0.17.7,
//! revision 503b4bd1). The model call itself (ONNX Runtime) is not here: this
//! module gives the windows to feed it and turns one window's logits into a
//! [`SegChunk`].
//!
//! What the Swift does after the model call, and what this port keeps:
//!
//! - The logits of each frame are renormalised with their log-sum-exp
//!   (lines 337-346), so `log_probs` are proper log-probabilities even when
//!   the model output is not.
//! - The speaker weights are NOT a marginalisation of the probabilities. Each
//!   frame takes the argmax class (first maximum wins, lines 327-335) and gives
//!   weight 1.0 to the local speakers of that powerset class and 0.0 to the
//!   others (lines 393-399, "Binary speaker activation from argmax"). The
//!   comment at lines 412-414 speaks of marginalising, but the code copies the
//!   binary activations.
//! - Nothing is trimmed: the windows that run past the end of the track are
//!   zero padded and keep all their frames. Reconstruction may therefore place
//!   speech after the end of the audio, if the model hears some in the padding.
//!
//! Not ported: the class histogram, probability statistics, speech-frame count
//! against `speechOnsetThreshold` and timing logs (lines 65-92, 360-391,
//! 401-403, 435-443, 453-525). They only feed debug logs.

use super::config::{
    LOCAL_SPEAKERS, POWERSET_CLASSES, SAMPLE_RATE, STEP_SAMPLES, WINDOW_SAMPLES, WINDOW_SECONDS,
};
use super::types::SegChunk;

/// The local speakers of each powerset class, in the order of the model's
/// output (`powerset`, `OfflineSegmentationProcessor.swift` lines 15-24).
/// The Swift table has an eighth entry `[0, 1, 2]`. The community-1 model has
/// 7 classes, so the argmax can never reach it and it is left out.
const POWERSET: [[f32; LOCAL_SPEAKERS]; POWERSET_CLASSES] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [1.0, 1.0, 0.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, 1.0],
];

/// The start sample of every window the model runs on a track of
/// `total_samples` samples.
///
/// The Swift loop is `stride(from: 0, to: totalSamples, by: stepSize)`
/// (line 190): windows start at 0, 32000, 64000, ... while the start is below
/// the end of the track. So the count is `ceil(total / step)`, the last window
/// starts less than one step before the end and therefore always covers the
/// last sample, and every window past the first that starts inside the last
/// 8 s is mostly zero padding but still runs. A track shorter than one window
/// gets one window; an empty track gets none (the Swift throws
/// `noSpeechDetected`, lines 51-53).
///
/// The progress total in `OfflineDiarizerManager.swift` line 216,
/// `max(1, ceil(total / step))`, agrees with this for every non-empty track;
/// for an empty one it says 1 where the processor runs nothing.
///
/// Size: one `usize` per window, 5400 for 3 hours.
pub fn window_starts(total_samples: usize) -> Vec<usize> {
    (0..total_samples).step_by(STEP_SAMPLES).collect()
}

/// The `WINDOW_SAMPLES` samples the model gets for the window that starts at
/// `start`: the track from `start`, then zeros up to the window size
/// (`populateWindow`, lines 118-187: the copy takes
/// `max(0, min(chunkSize, totalSamples - offset))` samples and
/// `vDSP_vclr` zeroes the rest). A `start` at or past the end gives all zeros.
/// The Swift reuses the overlapping part of the previous window by shifting;
/// the content is the same.
pub fn window_samples(track: &[f32], start: usize) -> Vec<f32> {
    let mut window: Vec<f32> = vec![0.0; WINDOW_SAMPLES];
    if start < track.len() {
        let available: usize = (track.len() - start).min(WINDOW_SAMPLES);
        window[..available].copy_from_slice(&track[start..start + available]);
    }
    window
}

/// Decode the output of the model for one window.
///
/// `logits` holds `frames x POWERSET_CLASSES` values, row-major (the ONNX
/// output `[1, 589, 7]` of one window). `start_sample` is the start of the
/// window on the track, as given by [`window_starts`]. If `logits` is shorter
/// than `frames` rows, only the complete rows are decoded and `frame_duration`
/// follows their count; an empty result has `frame_duration` 0, which
/// reconstruction treats as "no segmentation".
///
/// Per frame (Swift lines 321-405):
/// 1. the argmax class, first maximum on ties; NaN and `-inf` never win, so a
///    frame of them is class 0, silence (lines 327-335);
/// 2. `log_probs = logits - logsumexp(logits)` (lines 337-346);
/// 3. weights = the 0/1 membership of the argmax class (lines 393-399).
///
/// Size: with `Vec<Vec<f32>>` rows (24 bytes of header and an allocation per
/// frame) a window is about 33 KB of `log_probs` and 24 KB of weights. 5400
/// windows (3 hours) need about 300 MB if everything is kept, 130 MB for the
/// weights alone. Callers should empty `log_probs` once the embeddings of the
/// window are made; reconstruction only reads the weights.
pub fn decode_chunk(index: usize, start_sample: usize, logits: &[f32], frames: usize) -> SegChunk {
    let available_rows: usize = logits.len() / POWERSET_CLASSES;
    let rows: usize = frames.min(available_rows);

    let mut log_probs: Vec<Vec<f32>> = Vec::with_capacity(rows);
    let mut speaker_weights: Vec<Vec<f32>> = Vec::with_capacity(rows);

    let (complete_rows, _) = logits.as_chunks::<POWERSET_CLASSES>();
    for row in complete_rows.iter().take(rows) {
        let best: usize = argmax_first(row);
        let shift: f64 = log_sum_exp(row);
        log_probs.push(row.iter().map(|&x| (f64::from(x) - shift) as f32).collect());
        speaker_weights.push(POWERSET[best].to_vec());
    }

    let frame_duration: f64 = if rows > 0 {
        WINDOW_SECONDS / rows as f64
    } else {
        0.0
    };

    SegChunk {
        index,
        offset_seconds: start_sample as f64 / SAMPLE_RATE as f64,
        frame_duration,
        log_probs,
        speaker_weights,
    }
}

/// Index of the first maximum; starts at class 0 and only a strictly greater
/// value replaces it, from `-f32::MAX` as in lines 327-335.
fn argmax_first(row: &[f32]) -> usize {
    let mut best_index: usize = 0;
    let mut best_value: f32 = -f32::MAX;
    for (class, &value) in row.iter().enumerate() {
        if value > best_value {
            best_value = value;
            best_index = class;
        }
    }
    best_index
}

/// `log(sum(exp(x)))`, shifted by the maximum so it cannot overflow. The Swift
/// calls `VDSPOperations.logSumExp`, whose source is not in the spec; this is
/// the mathematical definition. When no value is finite the shift is 0 so the
/// log-probabilities stay what they were instead of becoming NaN.
fn log_sum_exp(row: &[f32]) -> f64 {
    let max: f64 = row
        .iter()
        .map(|&x| f64::from(x))
        .filter(|x| x.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    if !max.is_finite() {
        return 0.0;
    }
    let sum: f64 = row.iter().map(|&x| (f64::from(x) - max).exp()).sum();
    max + sum.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame whose logits favour `class` (0 for it, -10 for the rest).
    fn frame_for(class: usize) -> Vec<f32> {
        let mut row: Vec<f32> = vec![-10.0; POWERSET_CLASSES];
        row[class] = 0.0;
        row
    }

    #[test]
    fn each_powerset_class_gives_its_speakers() {
        // powerset table, OfflineSegmentationProcessor.swift lines 15-24,
        // weights from lines 393-399.
        let expected: [[f32; 3]; 7] = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        for (class, want) in expected.iter().enumerate() {
            let chunk: SegChunk = decode_chunk(0, 0, &frame_for(class), 1);
            assert_eq!(chunk.speaker_weights, vec![want.to_vec()], "class {class}");
        }
    }

    #[test]
    fn weights_are_binary_even_when_the_winner_is_uncertain() {
        // Class 4 ([0,1]) wins with probability ~0.4 only: the Swift still
        // gives 1.0 to both speakers (lines 393-399), no marginalisation.
        let row: Vec<f32> = vec![-1.2, -1.3, -1.3, -3.0, -0.9, -3.0, -3.0];
        let chunk: SegChunk = decode_chunk(0, 0, &row, 1);
        assert_eq!(chunk.speaker_weights, vec![vec![1.0, 1.0, 0.0]]);
    }

    #[test]
    fn first_maximum_wins_a_tie() {
        // Strict `>` at line 331: equal values keep the lower class.
        let all_equal: Vec<f32> = vec![-1.0; POWERSET_CLASSES];
        let chunk: SegChunk = decode_chunk(0, 0, &all_equal, 1);
        assert_eq!(chunk.speaker_weights, vec![vec![0.0, 0.0, 0.0]]);

        let mut tie: Vec<f32> = vec![-5.0; POWERSET_CLASSES];
        tie[2] = -1.0;
        tie[5] = -1.0;
        let chunk: SegChunk = decode_chunk(0, 0, &tie, 1);
        assert_eq!(chunk.speaker_weights, vec![vec![0.0, 1.0, 0.0]]);
    }

    #[test]
    fn nan_and_infinite_frames_are_silence_and_do_not_panic() {
        let nan: Vec<f32> = vec![f32::NAN; POWERSET_CLASSES];
        let neg_inf: Vec<f32> = vec![f32::NEG_INFINITY; POWERSET_CLASSES];
        for row in [nan, neg_inf] {
            let chunk: SegChunk = decode_chunk(0, 0, &row, 1);
            assert_eq!(chunk.speaker_weights, vec![vec![0.0, 0.0, 0.0]]);
            assert_eq!(chunk.log_probs.len(), 1);
        }
    }

    #[test]
    fn log_probs_are_renormalised_logits() {
        // Lines 337-346: logits minus their log-sum-exp. Equal logits of 3.0
        // over 7 classes give ln(1/7) each.
        let row: Vec<f32> = vec![3.0; POWERSET_CLASSES];
        let chunk: SegChunk = decode_chunk(0, 0, &row, 1);
        let want: f32 = (1.0_f64 / 7.0).ln() as f32;
        for value in &chunk.log_probs[0] {
            assert!((value - want).abs() < 1e-6, "{value} vs {want}");
        }

        // Already normalised input stays (almost) unchanged, and the
        // exponentials sum to 1.
        let normalised: Vec<f32> = vec![-0.5, -2.0, -3.0, -4.0, -5.0, -6.0, -7.0];
        let sum: f64 = normalised.iter().map(|&x| f64::from(x).exp()).sum();
        let log_norm: f32 = sum.ln() as f32;
        let chunk: SegChunk = decode_chunk(0, 0, &normalised, 1);
        for (got, input) in chunk.log_probs[0].iter().zip(&normalised) {
            assert!((got - (input - log_norm)).abs() < 1e-6);
        }
        let total: f64 = chunk.log_probs[0].iter().map(|&x| f64::from(x).exp()).sum();
        assert!((total - 1.0).abs() < 1e-6);
    }

    #[test]
    fn chunk_header_follows_the_window() {
        // offset = start / 16000 (line 303), frame = 10 s / frames (line 286).
        let logits: Vec<f32> = (0..589).flat_map(|_| frame_for(0)).collect();
        let chunk: SegChunk = decode_chunk(7, 64_000, &logits, 589);
        assert_eq!(chunk.index, 7);
        assert_eq!(chunk.offset_seconds, 4.0);
        assert_eq!(chunk.frame_duration, 10.0 / 589.0);
        assert_eq!(chunk.log_probs.len(), 589);
        assert_eq!(chunk.speaker_weights.len(), 589);
    }

    #[test]
    fn frames_decode_independently_in_row_major_order() {
        let mut logits: Vec<f32> = Vec::new();
        logits.extend(frame_for(0));
        logits.extend(frame_for(1));
        logits.extend(frame_for(6));
        let chunk: SegChunk = decode_chunk(0, 0, &logits, 3);
        assert_eq!(
            chunk.speaker_weights,
            vec![
                vec![0.0, 0.0, 0.0],
                vec![1.0, 0.0, 0.0],
                vec![0.0, 1.0, 1.0]
            ]
        );
    }

    #[test]
    fn short_or_empty_logits_decode_what_they_hold() {
        let empty: SegChunk = decode_chunk(0, 0, &[], 589);
        assert!(empty.speaker_weights.is_empty());
        assert_eq!(empty.frame_duration, 0.0);

        let zero_frames: SegChunk = decode_chunk(0, 0, &frame_for(1), 0);
        assert!(zero_frames.speaker_weights.is_empty());

        // 1.5 rows: one complete row.
        let mut partial: Vec<f32> = frame_for(2);
        partial.extend([0.0, 0.0, 0.0]);
        let chunk: SegChunk = decode_chunk(0, 0, &partial, 2);
        assert_eq!(chunk.speaker_weights.len(), 1);
        assert_eq!(chunk.frame_duration, 10.0);
    }

    #[test]
    fn window_starts_of_short_tracks() {
        // stride(from: 0, to: total, by: step), line 190.
        assert_eq!(window_starts(0), Vec::<usize>::new());
        assert_eq!(window_starts(1), vec![0]);
        assert_eq!(window_starts(100), vec![0]);
        assert_eq!(
            window_starts(WINDOW_SAMPLES - 1),
            vec![0, 32_000, 64_000, 96_000, 128_000]
        );
    }

    #[test]
    fn window_starts_at_the_window_and_step_boundaries() {
        // Exactly one window: 160000 samples still start a window every
        // 32000 while the start is below the end, so 5 windows.
        assert_eq!(
            window_starts(WINDOW_SAMPLES),
            vec![0, 32_000, 64_000, 96_000, 128_000]
        );
        // 1 window + 1 sample: a sixth window starts at 160000, which is
        // below 160001 (stride is exclusive of the end).
        assert_eq!(
            window_starts(WINDOW_SAMPLES + 1),
            vec![0, 32_000, 64_000, 96_000, 128_000, 160_000]
        );
        // Exactly one step: one window; one step + 1: two windows.
        assert_eq!(window_starts(STEP_SAMPLES), vec![0]);
        assert_eq!(window_starts(STEP_SAMPLES + 1), vec![0, 32_000]);
    }

    #[test]
    fn window_starts_count_matches_the_progress_formula() {
        // OfflineDiarizerManager.swift line 216: max(1, (n + step - 1) / step),
        // which is the ceiling division.
        for total in [1_usize, 31_999, 32_000, 32_001, 160_000, 160_001, 1_000_003] {
            let formula: usize = total.div_ceil(STEP_SAMPLES).max(1);
            assert_eq!(window_starts(total).len(), formula, "total {total}");
        }
    }

    #[test]
    fn three_hours_has_5400_windows_and_the_last_covers_the_end() {
        let total: usize = 3 * 3600 * SAMPLE_RATE;
        let starts: Vec<usize> = window_starts(total);
        assert_eq!(starts.len(), 5400);
        assert_eq!(starts[0], 0);
        let last: usize = starts[starts.len() - 1];
        assert_eq!(last, 5399 * STEP_SAMPLES);
        assert!(last < total);
        assert!(last + WINDOW_SAMPLES >= total);
        for pair in starts.windows(2) {
            assert_eq!(pair[1] - pair[0], STEP_SAMPLES);
        }
        // One more sample needs one more window.
        assert_eq!(window_starts(total + 1).len(), 5401);
    }

    #[test]
    fn window_samples_pads_with_zeros_past_the_end() {
        // Lines 122, 165-178: copy what is there, clear the rest.
        let track: Vec<f32> = (1..=100).map(|x| x as f32).collect();
        let window: Vec<f32> = window_samples(&track, 0);
        assert_eq!(window.len(), WINDOW_SAMPLES);
        assert_eq!(window[0], 1.0);
        assert_eq!(window[99], 100.0);
        assert!(window[100..].iter().all(|&x| x == 0.0));

        let tail: Vec<f32> = window_samples(&track, 90);
        assert_eq!(tail.len(), WINDOW_SAMPLES);
        assert_eq!(&tail[..10], &track[90..]);
        assert!(tail[10..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn window_samples_takes_a_full_window_from_a_long_track() {
        let track: Vec<f32> = (0..200_000).map(|x| x as f32).collect();
        let window: Vec<f32> = window_samples(&track, STEP_SAMPLES);
        assert_eq!(window[0], 32_000.0);
        assert_eq!(window[WINDOW_SAMPLES - 1], 191_999.0);

        let padded: Vec<f32> = window_samples(&track, 64_000);
        let available: usize = track.len() - 64_000;
        assert_eq!(padded[available - 1], 199_999.0);
        assert!(padded[available..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn window_samples_of_nothing_is_all_zeros() {
        assert!(window_samples(&[], 0).iter().all(|&x| x == 0.0));
        assert_eq!(window_samples(&[1.0], 5).len(), WINDOW_SAMPLES);
        assert!(window_samples(&[1.0], 5).iter().all(|&x| x == 0.0));
        assert!(window_samples(&[1.0], usize::MAX).iter().all(|&x| x == 0.0));
    }
}
