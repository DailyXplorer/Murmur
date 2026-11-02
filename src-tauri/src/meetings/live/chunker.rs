//! Cuts one track's audio, as the writer places it on the meeting timeline,
//! into pieces for the live transcript: at the first pause after
//! `CHUNK_TARGET`, or at `CHUNK_MAX` without one, the next piece then
//! starting `CHUNK_OVERLAP` earlier so the seam can be joined on the words
//! both heard. Pauses are the ones dictation cuts at (`audio_toolkit::pauses`).

use std::collections::VecDeque;

use crate::audio_toolkit::gain::{frame_levels, FRAME};
use crate::audio_toolkit::pauses::{noise_floor, MIN_PAUSE_FRAMES, PAUSE_OVER_NOISE_DB};
use crate::meetings::merge::Track as Source;

pub const RATE: usize = 16_000;
/// Pieces are cut at the first pause that reaches past this length.
pub const CHUNK_TARGET: usize = 6 * RATE;
/// Without a pause, the piece is cut here, inside speech.
pub const CHUNK_MAX: usize = 12 * RATE;
/// What a piece cut inside speech shares with the next one.
pub const CHUNK_OVERLAP: usize = 2 * RATE;
const SILENT_DBFS: f32 = -60.0;
const HISTORY_FRAMES: usize = 30 * RATE / FRAME;

/// Audio the writer just placed: `samples` from `start_sample` on.
pub struct LiveBlock<'a> {
    pub start_sample: u64,
    pub samples: &'a [f32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    /// At a pause: nothing of this piece is heard again.
    Pause,
    /// Inside speech: the next piece starts with the last `CHUNK_OVERLAP`.
    Hard,
    /// The track ended, or skipped ahead.
    End,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub source: Source,
    pub start_sample: u64,
    /// The first `overlap` samples were the end of the previous piece.
    pub overlap: usize,
    pub samples: Vec<f32>,
    pub cut: Cut,
}

impl Chunk {
    pub fn end_sample(&self) -> u64 {
        self.start_sample + self.samples.len() as u64
    }

    /// Nothing to send: a recogniser fed silence invents words.
    pub fn silent(&self) -> bool {
        frame_levels(&self.samples)
            .iter()
            .all(|level| *level < SILENT_DBFS)
    }

    /// Where its words become final: a later piece may still change the
    /// ones it shares with it.
    pub fn settled_sample(&self) -> u64 {
        match self.cut {
            Cut::Hard => self.end_sample() - CHUNK_OVERLAP as u64,
            Cut::Pause | Cut::End => self.end_sample(),
        }
    }
}

pub struct Chunker {
    source: Source,
    start: u64,
    samples: Vec<f32>,
    overlap: usize,
    levels: Vec<f32>,
    history: VecDeque<f32>,
}

impl Chunker {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            start: 0,
            samples: Vec::new(),
            overlap: 0,
            levels: Vec::new(),
            history: VecDeque::new(),
        }
    }

    /// Takes the next audio and returns the pieces it completed. Audio that
    /// does not follow on (the writer padded a gap with silence) ends the
    /// piece before it.
    pub fn push(&mut self, block: LiveBlock<'_>) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        if block.start_sample != self.start + self.samples.len() as u64 {
            chunks.extend(self.finish());
            self.start = block.start_sample;
        }
        self.samples.extend_from_slice(block.samples);
        let whole = self.samples.len() / FRAME;
        if whole > self.levels.len() {
            let fresh = frame_levels(&self.samples[self.levels.len() * FRAME..whole * FRAME]);
            for level in &fresh {
                if self.history.len() == HISTORY_FRAMES {
                    self.history.pop_front();
                }
                self.history.push_back(*level);
            }
            self.levels.extend(fresh);
        }
        while let Some((at, cut)) = self.cut() {
            chunks.push(self.take(at, cut));
        }
        chunks
    }

    /// The rest of the track, unless it holds only what the last piece
    /// already had.
    pub fn finish(&mut self) -> Option<Chunk> {
        let left = self.samples.len() > self.overlap;
        let chunk = left.then(|| self.take(self.samples.len(), Cut::End));
        self.start += self.samples.len() as u64;
        self.samples.clear();
        self.levels.clear();
        self.overlap = 0;
        chunk
    }

    fn cut(&self) -> Option<(usize, Cut)> {
        if self.samples.len() < CHUNK_TARGET {
            return None;
        }
        let history: Vec<f32> = self.history.iter().copied().collect();
        let quiet_below = noise_floor(&history) + PAUSE_OVER_NOISE_DB;
        let target = CHUNK_TARGET / FRAME;
        let mut run = 0;
        for (frame, level) in self.levels.iter().enumerate() {
            if *level < quiet_below {
                run += 1;
            } else {
                run = 0;
            }
            if run >= MIN_PAUSE_FRAMES && frame + 1 >= target {
                return Some(((frame + 1 - run / 2) * FRAME, Cut::Pause));
            }
        }
        (self.samples.len() >= CHUNK_MAX).then_some((CHUNK_MAX, Cut::Hard))
    }

    fn take(&mut self, at: usize, cut: Cut) -> Chunk {
        let keep_from = match cut {
            Cut::Hard => at - CHUNK_OVERLAP,
            Cut::Pause | Cut::End => at,
        };
        let rest = self.samples.split_off(keep_from);
        let mut samples = std::mem::replace(&mut self.samples, rest);
        if cut == Cut::Hard {
            samples.extend_from_slice(&self.samples[..CHUNK_OVERLAP]);
        }
        let chunk = Chunk {
            source: self.source,
            start_sample: self.start,
            overlap: self.overlap,
            samples,
            cut,
        };
        self.start += keep_from as u64;
        self.levels
            .drain(..(keep_from / FRAME).min(self.levels.len()));
        self.overlap = if cut == Cut::Hard { CHUNK_OVERLAP } else { 0 };
        chunk
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech(seconds: f32) -> Vec<f32> {
        let count = (seconds * RATE as f32) as usize;
        let syllable = RATE * 60 / 1_000;
        (0..count)
            .map(|i| {
                let level = if i % syllable < syllable * 2 / 3 {
                    0.1
                } else {
                    0.04
                };
                level * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin()
            })
            .collect()
    }

    fn pause(seconds: f32) -> Vec<f32> {
        let count = (seconds * RATE as f32) as usize;
        (0..count)
            .map(|i| 0.0005 * (2.0 * std::f32::consts::PI * 97.0 * i as f32 / RATE as f32).sin())
            .collect()
    }

    fn feed(chunker: &mut Chunker, audio: &[f32]) -> Vec<Chunk> {
        audio
            .chunks(170)
            .enumerate()
            .flat_map(|(index, samples)| {
                chunker.push(LiveBlock {
                    start_sample: (index * 170) as u64,
                    samples,
                })
            })
            .collect()
    }

    fn seconds(samples: u64) -> f32 {
        samples as f32 / RATE as f32
    }

    #[test]
    fn cuts_at_first_pause_after_target() {
        let audio = [
            speech(7.0),
            pause(0.5),
            speech(2.0),
            pause(0.5),
            speech(3.0),
        ]
        .concat();
        let mut chunker = Chunker::new(Source::Mic);
        let chunks = feed(&mut chunker, &audio);
        assert_eq!(
            chunks.len(),
            1,
            "{:?}",
            chunks.iter().map(|c| c.cut).collect::<Vec<_>>()
        );
        let cut = seconds(chunks[0].end_sample());
        assert!((7.0..7.5).contains(&cut), "cut at {cut} s");
        assert_eq!(chunks[0].cut, Cut::Pause);
        assert_eq!(chunks[0].overlap, 0);

        let last = chunker.finish().unwrap();
        assert_eq!(
            last.start_sample,
            chunks[0].end_sample(),
            "no audio lost or repeated"
        );
        assert_eq!(last.end_sample(), audio.len() as u64);
        assert_eq!(last.cut, Cut::End);
    }

    #[test]
    fn hard_cuts_at_max_with_overlap() {
        let audio = speech(30.0);
        let mut chunker = Chunker::new(Source::System);
        let chunks = feed(&mut chunker, &audio);
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|chunk| chunk.cut == Cut::Hard));
        assert_eq!(chunks[0].samples.len(), CHUNK_MAX);
        assert_eq!(chunks[0].overlap, 0);
        assert_eq!(
            chunks[1].start_sample,
            (CHUNK_MAX - CHUNK_OVERLAP) as u64,
            "the next piece hears the last 2 s again"
        );
        assert_eq!(chunks[1].overlap, CHUNK_OVERLAP);
        assert_eq!(
            chunks[1].samples[..CHUNK_OVERLAP],
            chunks[0].samples[CHUNK_MAX - CHUNK_OVERLAP..]
        );
        assert_eq!(chunks[0].settled_sample(), chunks[1].start_sample);
        let last = chunker.finish().unwrap();
        assert_eq!(last.end_sample(), audio.len() as u64);
        assert_eq!(last.overlap, CHUNK_OVERLAP);
    }

    #[test]
    fn ignores_pause_before_min() {
        let audio = [
            speech(1.0),
            pause(0.6),
            speech(7.4),
            pause(0.6),
            speech(1.0),
        ]
        .concat();
        let mut chunker = Chunker::new(Source::Mic);
        let chunks = feed(&mut chunker, &audio);
        assert_eq!(chunks.len(), 1);
        let cut = seconds(chunks[0].end_sample());
        assert!((9.0..9.6).contains(&cut), "cut at {cut} s");

        let mut chunker = Chunker::new(Source::Mic);
        let chunks = feed(&mut chunker, &[pause(6.5), speech(1.0)].concat());
        assert!(chunks.iter().all(|chunk| chunk.samples.len() >= 3 * RATE));
    }

    #[test]
    fn silent_chunk_is_not_sent() {
        let mut chunker = Chunker::new(Source::System);
        let chunks = feed(&mut chunker, &vec![0.0; 13 * RATE]);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(Chunk::silent));
        assert!(!feed(&mut Chunker::new(Source::Mic), &speech(13.0))[0].silent());
    }

    #[test]
    fn audio_after_a_gap_starts_a_new_piece() {
        let mut chunker = Chunker::new(Source::Mic);
        let first = speech(2.0);
        assert!(chunker
            .push(LiveBlock {
                start_sample: 1_600,
                samples: &first,
            })
            .is_empty());
        let after = chunker.push(LiveBlock {
            start_sample: 1_600 + first.len() as u64 + 8_000,
            samples: &first,
        });
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].start_sample, 1_600);
        assert_eq!(after[0].cut, Cut::End);
        let last = chunker.finish().unwrap();
        assert_eq!(last.start_sample, 1_600 + first.len() as u64 + 8_000);
    }
}
