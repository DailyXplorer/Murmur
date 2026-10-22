//! Cuts a long dictation at pauses so each piece fits one pass of the local
//! engine. Pieces never overlap and never end inside a word: an engine's own
//! long-audio mode overlaps fixed windows, and its seams drop or repeat words.
//! Levels are the 20 ms frame energies meetings already use.

use super::constants::TARGET_SAMPLE_RATE;
use super::gain::{frame_levels, DIGITAL_SILENCE_DBFS, FRAME};
use crate::meetings::chunking::speech_threshold;
use std::ops::Range;

/// A cut is searched for in the last 6 s before a piece's maximum length.
const SEARCH_FRAMES: usize = 6 * TARGET_SAMPLE_RATE as usize / FRAME;
/// Frames up to this far above the clip's noise floor are a pause.
const PAUSE_OVER_NOISE_DB: f32 = 6.0;
/// 160 ms, the shortest run of quiet frames that counts as a pause.
const MIN_PAUSE_FRAMES: usize = 8;
/// Without a pause, the cut lands in the quietest 100 ms.
const QUIETEST_FRAMES: usize = 5;
/// Less speech than 200 ms is no speech.
const MIN_SPEECH_FRAMES: usize = 10;

/// The noise floor: the 10th percentile of the frames that hold any signal.
fn noise_floor(levels: &[f32]) -> f32 {
    let mut sorted: Vec<f32> = levels
        .iter()
        .copied()
        .filter(|level| *level > DIGITAL_SILENCE_DBFS)
        .collect();
    if sorted.is_empty() {
        return DIGITAL_SILENCE_DBFS;
    }
    sorted.sort_by(f32::total_cmp);
    sorted[(sorted.len() - 1) / 10]
}

/// Splits 16 kHz `samples` into contiguous pieces of at most `max_len`
/// samples. A clip that fits is one piece; an empty clip has none.
pub fn split_at_pauses(samples: &[f32], max_len: usize) -> Vec<Range<usize>> {
    let max_frames = (max_len / FRAME).max(1);
    let levels = frame_levels(samples);
    let quiet_below = noise_floor(&levels) + PAUSE_OVER_NOISE_DB;
    let mut pieces = Vec::new();
    let mut start = 0;
    while samples.len() - start > max_len {
        let cut = cut_frame(&levels, quiet_below, start / FRAME, max_frames) * FRAME;
        pieces.push(start..cut);
        start = cut;
    }
    if start < samples.len() {
        pieces.push(start..samples.len());
    }
    pieces
}

/// The frame boundary that ends the piece starting at frame `first`: the
/// middle of the longest pause in the search window, the latest on ties, or
/// else the middle of its quietest 100 ms. Always after `first` and at most
/// `max_frames` from it.
fn cut_frame(levels: &[f32], quiet_below: f32, first: usize, max_frames: usize) -> usize {
    let end = first + max_frames;
    let from = end.saturating_sub(SEARCH_FRAMES).max(first + 1);

    let mut longest = (0, from);
    let mut run = 0;
    for (frame, level) in levels.iter().enumerate().take(end).skip(from) {
        if *level < quiet_below {
            run += 1;
            if run >= longest.0 {
                longest = (run, frame + 1 - run);
            }
        } else {
            run = 0;
        }
    }
    if longest.0 >= MIN_PAUSE_FRAMES {
        return longest.1 + longest.0 / 2;
    }

    if end < from + QUIETEST_FRAMES {
        return end;
    }
    let mut quietest = (f32::INFINITY, from);
    for window in from..=end - QUIETEST_FRAMES {
        let energy: f32 = levels[window..window + QUIETEST_FRAMES].iter().sum();
        if energy <= quietest.0 {
            quietest = (energy, window);
        }
    }
    quietest.1 + QUIETEST_FRAMES / 2
}

/// Whether a clip holds no speech, judged by the threshold meetings use. The
/// engine is not run on it: a recogniser fed silence invents words.
pub fn is_silent(samples: &[f32]) -> bool {
    let levels = frame_levels(samples);
    match speech_threshold(&levels) {
        None => true,
        Some(threshold) => {
            levels.iter().filter(|level| **level >= threshold).count() < MIN_SPEECH_FRAMES
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEECH: f32 = -22.0;
    const ROOM: f32 = -60.0;
    const SECOND: usize = TARGET_SAMPLE_RATE as usize;

    fn samples_for_ms(ms: usize) -> usize {
        ms * SECOND / 1_000
    }

    /// A 440 Hz tone whose level follows `segments` of `(ms, dBFS)`.
    fn clip(segments: &[(usize, f32)]) -> Vec<f32> {
        let mut samples = Vec::new();
        for (ms, dbfs) in segments {
            let amplitude = std::f32::consts::SQRT_2 * 10f32.powf(dbfs / 20.0);
            for _ in 0..samples_for_ms(*ms) {
                let t = samples.len() as f32 / SECOND as f32;
                samples.push(amplitude * (2.0 * std::f32::consts::PI * 440.0 * t).sin());
            }
        }
        samples
    }

    /// Speech at syllable rhythm, loud then softer frames, with no pause.
    fn talking(ms: usize) -> Vec<(usize, f32)> {
        (0..ms / 60)
            .flat_map(|_| [(40, SPEECH), (20, SPEECH - 10.0)])
            .collect()
    }

    fn at_ms(sample: usize) -> usize {
        sample * 1_000 / SECOND
    }

    fn assert_covers(pieces: &[Range<usize>], len: usize, max_len: usize) {
        assert_eq!(pieces.first().map(|piece| piece.start), Some(0));
        assert_eq!(pieces.last().map(|piece| piece.end), Some(len));
        for pair in pieces.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "{pieces:?}");
        }
        for piece in pieces {
            assert!(!piece.is_empty(), "{pieces:?}");
            assert!(piece.len() <= max_len, "{pieces:?}");
        }
    }

    #[test]
    fn a_clip_under_the_limit_is_one_piece() {
        let samples = clip(&talking(9_000));
        assert_eq!(
            split_at_pauses(&samples, samples_for_ms(14_500)),
            vec![0..samples.len()]
        );
        let exact = samples_for_ms(9_000);
        assert_eq!(split_at_pauses(&samples, exact), vec![0..samples.len()]);
        assert!(split_at_pauses(&[], samples_for_ms(14_500)).is_empty());
    }

    #[test]
    fn the_cut_lands_inside_the_longest_pause_near_the_limit() {
        let mut segments = Vec::new();
        segments.extend(talking(3_000));
        // A long pause, but too early: it would leave a short first piece.
        segments.push((1_000, ROOM));
        segments.extend(talking(6_000));
        segments.push((300, ROOM));
        segments.extend(talking(1_680));
        segments.push((600, ROOM));
        segments.extend(talking(8_040));
        let samples = clip(&segments);

        let pieces = split_at_pauses(&samples, samples_for_ms(14_500));
        assert_covers(&pieces, samples.len(), samples_for_ms(14_500));
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        let cut = at_ms(pieces[0].end);
        assert!((12_000..12_600).contains(&cut), "cut at {cut} ms");
    }

    #[test]
    fn equal_pauses_cut_at_the_latest() {
        let mut segments = Vec::new();
        segments.extend(talking(9_000));
        segments.push((400, ROOM));
        segments.extend(talking(3_000));
        segments.push((400, ROOM));
        segments.extend(talking(6_000));
        let samples = clip(&segments);

        let pieces = split_at_pauses(&samples, samples_for_ms(14_500));
        let cut = at_ms(pieces[0].end);
        assert!((12_400..12_800).contains(&cut), "cut at {cut} ms");
    }

    #[test]
    fn without_a_pause_the_cut_lands_in_the_quietest_moment() {
        let mut segments = Vec::new();
        segments.extend(talking(11_040));
        // A dip shorter than a pause, between two words.
        segments.push((100, SPEECH - 30.0));
        segments.extend(talking(9_000));
        let samples = clip(&segments);

        let pieces = split_at_pauses(&samples, samples_for_ms(14_500));
        assert_covers(&pieces, samples.len(), samples_for_ms(14_500));
        let cut = at_ms(pieces[0].end);
        assert!((11_040..11_140).contains(&cut), "cut at {cut} ms");
    }

    #[test]
    fn no_piece_is_ever_longer_than_the_limit() {
        let mut state: u32 = 11;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 8
        };
        for _ in 0..30 {
            let max_len = samples_for_ms(1_000 + (next() % 20_000) as usize);
            let segments: Vec<(usize, f32)> = (0..next() % 120)
                .map(|_| {
                    (
                        20 * (1 + next() % 40) as usize,
                        -90.0 + (next() % 80) as f32,
                    )
                })
                .collect();
            let mut samples = clip(&segments);
            samples.extend(std::iter::repeat_n(0.0, (next() % 300) as usize));
            let pieces = split_at_pauses(&samples, max_len);
            if samples.is_empty() {
                assert!(pieces.is_empty());
                continue;
            }
            assert_covers(&pieces, samples.len(), max_len);
            assert_eq!(pieces, split_at_pauses(&samples, max_len));
        }
    }

    #[test]
    fn a_steady_tone_is_still_cut_within_the_limit() {
        let samples = clip(&[(40_000, SPEECH)]);
        let pieces = split_at_pauses(&samples, samples_for_ms(14_500));
        assert_covers(&pieces, samples.len(), samples_for_ms(14_500));
    }

    #[test]
    fn silence_and_room_noise_hold_no_speech() {
        assert!(is_silent(&[]));
        assert!(is_silent(&vec![0.0; 3 * SECOND]));
        assert!(is_silent(&clip(&[(3_000, ROOM)])));
        // A 100 ms click in a quiet room is not a dictation.
        assert!(is_silent(&clip(&[
            (1_500, ROOM),
            (100, SPEECH),
            (1_400, ROOM)
        ])));
    }

    #[test]
    fn short_or_continuous_speech_is_not_silent() {
        assert!(!is_silent(&clip(&[
            (1_000, ROOM),
            (400, SPEECH),
            (1_000, ROOM)
        ])));
        assert!(!is_silent(&clip(&talking(5_000))));
        assert!(!is_silent(&clip(&[(2_000, SPEECH)])));
    }
}
