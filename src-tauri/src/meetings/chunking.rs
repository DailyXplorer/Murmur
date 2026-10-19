//! Splits a track into pieces Codex accepts. Each piece stays well under the
//! 25 MB upload cap and is cut in the quietest moment near its end, so no word
//! is split and the pieces need no overlap. Pieces without speech are marked
//! so they are never uploaded: that saves requests, and a recogniser fed
//! silence tends to invent words.

use crate::audio_toolkit::gain::DIGITAL_SILENCE_DBFS;

/// Every level covers 20 ms of audio.
pub const FRAME_MS: u64 = 20;
/// 19.2 MB of 16 kHz WAV, a quarter under the upload cap.
pub const TARGET_MAX_MS: u64 = 10 * 60_000;
/// Cuts are searched for in the last two minutes before the maximum.
const SEARCH_FROM_MS: u64 = 8 * 60_000;
/// A piece shorter than this after a cut is balanced with the one before.
const MIN_TAIL_MS: u64 = 60_000;
/// The quietest stretch of this length is where a cut lands.
const QUIET_WINDOW_FRAMES: usize = 20;
/// Less speech than this in a piece is treated as silence.
pub const MIN_VOICED_MS: u64 = 1_500;
/// A piece Codex refuses as too large is halved, down to this length.
pub const MIN_SPLIT_MS: u64 = 60_000;
/// Speech stands this far above the noise floor.
const SPEECH_OVER_NOISE_DB: f32 = 10.0;
/// Bounds on the speech threshold: a near-silent track does not count
/// faint hiss as speech, and a noisy room does not hide normal speech.
const MIN_SPEECH_DBFS: f32 = -60.0;
const MAX_SPEECH_DBFS: f32 = -45.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkPlan {
    pub start_ms: u64,
    pub end_ms: u64,
    pub voiced_ms: u64,
}

impl ChunkPlan {
    pub fn silent(&self) -> bool {
        self.voiced_ms < MIN_VOICED_MS
    }
}

/// The level above which a frame is speech: the noise floor (the 20th
/// percentile of the frames that hold any signal) plus a margin, kept within
/// fixed bounds. `None` when the track holds no signal at all.
pub fn speech_threshold(levels: &[f32]) -> Option<f32> {
    let mut sorted: Vec<f32> = levels
        .iter()
        .copied()
        .filter(|level| *level > DIGITAL_SILENCE_DBFS)
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[(sorted.len() - 1) / 5];
    Some((floor + SPEECH_OVER_NOISE_DB).clamp(MIN_SPEECH_DBFS, MAX_SPEECH_DBFS))
}

/// System audio that is exact digital zero from start to end while the user
/// spoke: macOS gives a process tap only zeros when Murmur may not record
/// system audio, and there is no public way to ask beforehand. Nothing
/// played on the Mac gives zeros too, which the caller rules out.
pub fn system_audio_missing(mic: &[f32], system: &[f32]) -> bool {
    let mic_ms = mic.len() as u64 * FRAME_MS;
    !system.is_empty()
        && speech_threshold(system).is_none()
        && voiced_ms(mic, speech_threshold(mic), 0, mic_ms) >= MIN_VOICED_MS
}

fn frame_of(ms: u64) -> usize {
    (ms / FRAME_MS) as usize
}

/// Milliseconds of `[start_ms, end_ms)` at or above `threshold`.
pub fn voiced_ms(levels: &[f32], threshold: Option<f32>, start_ms: u64, end_ms: u64) -> u64 {
    let Some(threshold) = threshold else {
        return 0;
    };
    let end = frame_of(end_ms.div_ceil(FRAME_MS) * FRAME_MS).min(levels.len());
    let start = frame_of(start_ms).min(end);
    levels[start..end]
        .iter()
        .filter(|level| **level >= threshold)
        .count() as u64
        * FRAME_MS
}

/// The centre of the quietest `QUIET_WINDOW_FRAMES` stretch that starts in
/// `[from_ms, to_ms)`, the latest on ties, in milliseconds. Frames past the
/// recorded levels count as silence.
fn quietest_cut(levels: &[f32], from_ms: u64, to_ms: u64) -> u64 {
    let level = |frame: usize| levels.get(frame).copied().unwrap_or(DIGITAL_SILENCE_DBFS);
    let first = frame_of(from_ms);
    let last = frame_of(to_ms).max(first + 1);
    let mut sum: f32 = (first..first + QUIET_WINDOW_FRAMES).map(level).sum();
    let mut best = (sum, first);
    for start in first + 1..last {
        sum += level(start + QUIET_WINDOW_FRAMES - 1) - level(start - 1);
        if sum <= best.0 {
            best = (sum, start);
        }
    }
    (best.1 + QUIET_WINDOW_FRAMES / 2) as u64 * FRAME_MS
}

/// Splits `[0, total_ms)` of a track with these 20 ms `levels` into
/// contiguous pieces of at most `TARGET_MAX_MS`.
pub fn plan_chunks(levels: &[f32], total_ms: u64) -> Vec<ChunkPlan> {
    let threshold = speech_threshold(levels);
    let mut chunks = Vec::new();
    let mut cursor = 0;
    while cursor < total_ms {
        let remaining = total_ms - cursor;
        let end = if remaining <= TARGET_MAX_MS {
            total_ms
        } else {
            let cut = quietest_cut(
                levels,
                cursor + SEARCH_FROM_MS,
                cursor + TARGET_MAX_MS - QUIET_WINDOW_FRAMES as u64 * FRAME_MS,
            );
            if total_ms - cut < MIN_TAIL_MS {
                // Two pieces of similar length instead, still cut in a pause.
                split_chunk(levels, cursor, total_ms).map_or(cut, |(_, middle, _)| middle)
            } else {
                cut
            }
        };
        chunks.push(ChunkPlan {
            start_ms: cursor,
            end_ms: end,
            voiced_ms: voiced_ms(levels, threshold, cursor, end),
        });
        cursor = end;
    }
    chunks
}

/// Halves `[start_ms, end_ms)` at the quietest moment of its middle half,
/// for a piece Codex refused as too large or a plan's last two pieces.
/// `None` when the halves would be
/// shorter than `MIN_SPLIT_MS`.
pub fn split_chunk(levels: &[f32], start_ms: u64, end_ms: u64) -> Option<(u64, u64, u64)> {
    let length = end_ms.saturating_sub(start_ms);
    if length < 2 * MIN_SPLIT_MS {
        return None;
    }
    let half_window = QUIET_WINDOW_FRAMES as u64 * FRAME_MS / 2;
    let from = (start_ms + length / 4).max(start_ms + MIN_SPLIT_MS) - half_window;
    let to = (end_ms - length / 4).min(end_ms - MIN_SPLIT_MS) - half_window;
    let cut = quietest_cut(levels, from, to.max(from + FRAME_MS));
    Some((start_ms, cut, end_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEECH: f32 = -22.0;
    const ROOM: f32 = -58.0;
    const MINUTE: u64 = 60_000;

    fn frames(ms: u64) -> usize {
        (ms / FRAME_MS) as usize
    }

    /// Speech everywhere, with quiet gaps `(start_ms, length_ms)`.
    fn speech_with_gaps(total_ms: u64, gaps: &[(u64, u64)]) -> Vec<f32> {
        let mut levels = vec![SPEECH; frames(total_ms)];
        for (start, length) in gaps {
            for level in &mut levels[frames(*start)..frames(start + length)] {
                *level = ROOM;
            }
        }
        // A room-noise floor, so the threshold sits between the two.
        for level in levels.iter_mut().step_by(4) {
            *level = ROOM;
        }
        levels
    }

    fn assert_covers(chunks: &[ChunkPlan], total_ms: u64) {
        assert_eq!(chunks.first().map(|c| c.start_ms), Some(0));
        assert_eq!(chunks.last().map(|c| c.end_ms), Some(total_ms));
        for pair in chunks.windows(2) {
            assert_eq!(pair[0].end_ms, pair[1].start_ms, "{chunks:?}");
        }
        for chunk in chunks {
            assert!(chunk.end_ms > chunk.start_ms, "{chunk:?}");
            assert!(chunk.end_ms - chunk.start_ms <= TARGET_MAX_MS, "{chunk:?}");
        }
    }

    #[test]
    fn a_short_track_is_one_chunk() {
        let total = 4 * MINUTE + 7_310;
        let chunks = plan_chunks(&speech_with_gaps(total, &[]), total);
        assert_eq!(chunks.len(), 1);
        assert_eq!((chunks[0].start_ms, chunks[0].end_ms), (0, total));
        assert!(!chunks[0].silent());
    }

    #[test]
    fn a_long_track_is_cut_in_the_silence_near_the_limit() {
        let total = 25 * MINUTE;
        let levels = speech_with_gaps(total, &[(9 * MINUTE, 1_000), (18 * MINUTE, 600)]);
        let chunks = plan_chunks(&levels, total);
        assert_covers(&chunks, total);
        assert_eq!(chunks.len(), 3);
        let cut = chunks[0].end_ms;
        assert!(
            (9 * MINUTE..9 * MINUTE + 1_000).contains(&cut),
            "cut at {cut}"
        );
        let cut = chunks[1].end_ms;
        assert!(
            (18 * MINUTE..18 * MINUTE + 600).contains(&cut),
            "cut at {cut}"
        );
    }

    #[test]
    fn without_any_pause_the_cut_stays_within_the_limit() {
        let total = 21 * MINUTE;
        let levels = vec![SPEECH; frames(total)];
        let chunks = plan_chunks(&levels, total);
        assert_covers(&chunks, total);
        assert!(chunks[0].end_ms >= SEARCH_FROM_MS);
    }

    #[test]
    fn a_short_tail_is_balanced_with_the_piece_before_and_still_cut_in_a_pause() {
        // A cut at 10 minutes would leave 30 s; the only pause in the middle
        // of the track is at 4 minutes.
        let total = 10 * MINUTE + 30_000;
        let levels = speech_with_gaps(total, &[(4 * MINUTE, 800), (9 * MINUTE + 59_000, 800)]);
        let chunks = plan_chunks(&levels, total);
        assert_covers(&chunks, total);
        assert_eq!(chunks.len(), 2);
        let cut = chunks[0].end_ms;
        assert!(
            (4 * MINUTE..4 * MINUTE + 800).contains(&cut),
            "cut at {cut}"
        );
        assert!(total - cut >= MIN_TAIL_MS);
    }

    #[test]
    fn pieces_without_speech_are_silent() {
        let total = 25 * MINUTE;
        let mut levels = speech_with_gaps(total, &[]);
        // The first ten minutes hold only room noise and one cough.
        for level in &mut levels[..frames(10 * MINUTE)] {
            *level = ROOM;
        }
        for level in &mut levels[frames(MINUTE)..frames(MINUTE + 400)] {
            *level = SPEECH;
        }
        let chunks = plan_chunks(&levels, total);
        assert!(chunks[0].silent(), "{:?}", chunks[0]);
        assert!(chunks[1..].iter().all(|chunk| !chunk.silent()));
    }

    #[test]
    fn a_track_of_digital_silence_is_entirely_silent() {
        let total = 12 * MINUTE;
        let levels = vec![-180.0; frames(total)];
        let chunks = plan_chunks(&levels, total);
        assert_covers(&chunks, total);
        assert!(chunks.iter().all(ChunkPlan::silent));
        assert_eq!(speech_threshold(&levels), None);
    }

    #[test]
    fn system_audio_is_missing_only_when_it_is_digital_silence_while_the_mic_has_speech() {
        let total = 3 * MINUTE;
        let speech = speech_with_gaps(total, &[(MINUTE, 2_000)]);
        let zeros = vec![-180.0; frames(total)];
        let quiet_call = vec![ROOM; frames(total)];

        assert!(system_audio_missing(&speech, &zeros));
        // A call nobody spoke in still carries some signal.
        assert!(!system_audio_missing(&speech, &quiet_call));
        assert!(!system_audio_missing(&speech, &speech));
        // Nothing was said at all: no reason to suspect the permission.
        assert!(!system_audio_missing(&zeros, &zeros));
        assert!(!system_audio_missing(&quiet_call, &zeros));
        assert!(!system_audio_missing(&speech, &[]));
    }

    #[test]
    fn levels_shorter_than_the_track_never_panic() {
        let total = 23 * MINUTE;
        let chunks = plan_chunks(&[SPEECH; 10], total);
        assert_covers(&chunks, total);
        assert!(plan_chunks(&[], 0).is_empty());
    }

    #[test]
    fn plans_cover_random_tracks_and_are_deterministic() {
        let mut state: u32 = 7;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 8
        };
        for _ in 0..40 {
            let total = u64::from(next() % (95 * 60)) * 1_000 + u64::from(next() % 1_000);
            let levels: Vec<f32> = (0..frames(total) + 1)
                .map(|_| -90.0 + (next() % 80) as f32)
                .collect();
            let chunks = plan_chunks(&levels, total);
            if total == 0 {
                assert!(chunks.is_empty());
                continue;
            }
            assert_covers(&chunks, total);
            assert_eq!(chunks, plan_chunks(&levels, total));
        }
    }

    #[test]
    fn a_refused_piece_is_halved_in_its_quietest_middle_moment() {
        let start = 30 * MINUTE;
        let end = start + 10 * MINUTE;
        let levels = speech_with_gaps(end, &[(start + 4 * MINUTE, 500)]);
        let (from, cut, to) = split_chunk(&levels, start, end).unwrap();
        assert_eq!((from, to), (start, end));
        assert!(
            (start + 4 * MINUTE..start + 4 * MINUTE + 500).contains(&cut),
            "cut at {cut}"
        );
        assert!(split_chunk(&levels, start, start + 2 * MINUTE - 20).is_none());
        let (_, cut, _) = split_chunk(&levels, start, start + 2 * MINUTE).unwrap();
        assert_eq!(cut, start + MINUTE);
    }
}
