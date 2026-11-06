//! Speaker diarization with pyannote Community-1, for the platforms that have
//! no meetings helper (Windows). The Mac runs the offline pipeline of
//! FluidAudio 0.17.7 (Apache-2.0) through Core ML; this is the same pipeline in
//! plain Rust with the same weights and parameters (`config`), where ONNX
//! Runtime runs only the two neural networks (`pyannote`):
//!
//! 1. `segmentation`: windows of 10 s every 2 s, a powerset network says which
//!    of 3 local speakers speak in each of 589 frames.
//! 2. `fbank` and `embedding`: for each window and local speaker, the
//!    embedding of the frames only that speaker uses (ResNet, masked
//!    statistics pooling, linear layer), then `plda` to 128 values.
//! 3. `assign`, with `ahc` and `vbx`: all embeddings of the track are
//!    clustered into global speakers, and each local speaker takes a cluster
//!    (two of one window never share one).
//! 4. `reconstruct`: the windows vote on each frame, giving one speaker at a
//!    time and segments of at least a second.
//!
//! Every stage but the networks is plain Rust over slices, tested on any
//! platform. The local speakers of a window are not capped at the model's 3 by
//! this: the number of global speakers is whatever the clustering finds.

pub mod ahc;
pub mod assign;
pub mod config;
pub mod embedding;
pub mod fbank;
#[cfg(test)]
mod parity;
pub mod plda;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub mod pyannote;
pub mod reconstruct;
pub mod segmentation;
pub mod types;
pub mod vbx;

use std::collections::HashMap;

use config::{POWERSET_CLASSES, SEGMENTATION_FRAMES, WINDOW_SAMPLES};
use embedding::{Encoder, Extractor};
use types::{DiarSegment, SegChunk, Segmentation, StageError};

/// The segmentation network (`segmentation-community-1.onnx`).
pub trait SegmentationModel {
    /// `windows` holds `count` windows of `WINDOW_SAMPLES` samples. The answer
    /// holds `count x SEGMENTATION_FRAMES x POWERSET_CLASSES` log-probabilities,
    /// row major (`[B, 589, 7]`).
    fn segment(&mut self, windows: &[f32], count: usize) -> Result<Vec<f32>, StageError>;
}

/// Share of the work done after the windows are segmented and embedded: what
/// is left is the clustering and the reconstruction, a few seconds.
const WINDOWS_SHARE: f64 = 0.95;

/// Who spoke when on `track` (16 kHz mono), as segments of at most one
/// speaker at a time, in time order. Speakers are numbered from 0 in the
/// order they first speak. `extractor` is consumed: it holds the PLDA
/// transform and the linear layer of the embedding model, and `psi` is the
/// PLDA `psi` the clustering needs.
///
/// A track with no speech gives no segments. `stop` is read between groups of
/// windows. `progress` gets the share of the windows done (0.0 to 0.95).
pub fn diarize(
    track: &[f32],
    segmenter: &mut dyn SegmentationModel,
    encoder: &mut dyn Encoder,
    mut extractor: Extractor,
    psi: &[f64],
    stop: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(f64),
) -> Result<Vec<DiarSegment>, StageError> {
    let starts = segmentation::window_starts(track.len());
    let mut kept = Segmentation {
        chunks: Vec::with_capacity(starts.len()),
        num_frames: SEGMENTATION_FRAMES,
        frame_duration: config::WINDOW_SECONDS / SEGMENTATION_FRAMES as f64,
    };
    for (group_index, group) in starts.chunks(config::EMBEDDING_BATCH).enumerate() {
        if stop() {
            return Err(StageError::Stopped);
        }
        let mut windows: Vec<f32> = Vec::with_capacity(group.len() * WINDOW_SAMPLES);
        for &start in group {
            windows.extend(segmentation::window_samples(track, start));
        }
        let logits = segmenter.segment(&windows, group.len())?;
        drop(windows);
        let per_window = SEGMENTATION_FRAMES * POWERSET_CLASSES;
        if logits.len() != group.len() * per_window {
            return Err(StageError::Invalid("segmentation output size"));
        }
        let first = group_index * config::EMBEDDING_BATCH;
        let chunks: Vec<SegChunk> = group
            .iter()
            .enumerate()
            .map(|(offset, &start)| {
                segmentation::decode_chunk(
                    first + offset,
                    start,
                    &logits[offset * per_window..(offset + 1) * per_window],
                    SEGMENTATION_FRAMES,
                )
            })
            .collect();
        extractor.process(track, &chunks, encoder, stop)?;
        // Reconstruction needs the speaker weights only.
        kept.chunks.extend(chunks.into_iter().map(|mut chunk| {
            chunk.log_probs = Vec::new();
            chunk
        }));
        progress(WINDOWS_SHARE * (first + group.len()) as f64 / starts.len() as f64);
    }
    if kept.chunks.is_empty() {
        return Ok(Vec::new());
    }
    let embeddings = extractor.finish();
    if embeddings.is_empty() {
        return Ok(Vec::new());
    }
    let clustered = assign::cluster_embeddings(&embeddings, psi, kept.chunks.len())?;
    drop(embeddings);
    let segments =
        reconstruct::build_segments(&kept, &clustered.chunk_assignments, &clustered.centroids);
    Ok(by_first_appearance(segments))
}

/// The segments in time order with speakers numbered from 0 in the order they
/// first speak, so the numbering depends on the audio and nothing else.
pub fn by_first_appearance(mut segments: Vec<DiarSegment>) -> Vec<DiarSegment> {
    segments.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.end.total_cmp(&b.end))
            .then(a.speaker.cmp(&b.speaker))
    });
    let mut numbers: HashMap<usize, usize> = HashMap::new();
    for segment in &mut segments {
        let next = numbers.len();
        segment.speaker = *numbers.entry(segment.speaker).or_insert(next);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(speaker: usize, start: f64, end: f64) -> DiarSegment {
        DiarSegment {
            speaker,
            start,
            end,
        }
    }

    #[test]
    fn speakers_are_numbered_in_the_order_they_first_speak() {
        let numbered = by_first_appearance(vec![
            segment(7, 12.0, 15.0),
            segment(3, 0.0, 4.0),
            segment(9, 5.0, 8.0),
            segment(3, 9.0, 11.0),
        ]);
        let shown: Vec<(usize, f64)> = numbered.iter().map(|s| (s.speaker, s.start)).collect();
        assert_eq!(shown, [(0, 0.0), (1, 5.0), (0, 9.0), (2, 12.0)]);
    }

    #[test]
    fn the_numbering_does_not_depend_on_the_order_the_segments_come_in() {
        let segments = vec![
            segment(2, 3.0, 5.0),
            segment(1, 0.0, 2.0),
            segment(2, 6.0, 7.0),
        ];
        let mut reversed = segments.clone();
        reversed.reverse();
        assert_eq!(by_first_appearance(segments), by_first_appearance(reversed));
    }

    #[test]
    fn no_segments_stay_none() {
        assert!(by_first_appearance(Vec::new()).is_empty());
    }
}
