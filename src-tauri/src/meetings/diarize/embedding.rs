//! Speaker embeddings: turns the segmentation of windows into one 256-value
//! embedding (and its PLDA features) for each local speaker who spoke enough
//! in a window. Re-implements `OfflineEmbeddingExtractor.swift` and the
//! `Embedding` Core ML graph (`mil/embedding.mil`) of FluidAudio 0.17.7.
//!
//! What runs where. The ResNet encoder is behind [`Encoder`] (ONNX Runtime in
//! the app); everything else is plain Rust here: the fbank
//! ([`super::fbank`]), the speaker masks, the masked statistics pooling, the
//! final linear layer ([`Head`]), the L2 norm and the PLDA transform
//! ([`super::plda`]).
//!
//! One encoder pass per window, not per speaker. The Core ML model takes one
//! row per (window, local speaker) with the same fbank, but the mask enters
//! `embedding.mil` only at the pooling (`weights_3` is first used by
//! `weights_fp32`, after the last convolution), so the ResNet output of a
//! window is the same for its three speakers and is computed once. A window
//! with no usable speaker is not encoded at all.
//!
//! Not ported: `embedSpan` and the zero-vote re-embedding (both off by
//! default, they belong to a post-pass), and the skip strategy
//! `.maskSimilarity` (the default is `.none`: every speaker is embedded).
//!
//! Memory. The encoder output of one batch of 32 windows is 32 x 2560 x 125 x
//! 4 B = 41 MB and is dropped when the batch is done. What grows with the
//! track is [`Extractor`]'s list of [`TimedEmbedding`]s, about 4.4 KB each (589
//! `frame_weights` f32 = 2.4 KB, 256 f32 = 1 KB, 128 f64 = 1 KB): at most 3
//! per window, so 5400 per hour of audio at the very worst (24 MB) and a
//! fraction of that in a conversation.

use super::config::{
    EMBEDDING_BATCH, EMBEDDING_DIM, ENCODER_CHANNELS, ENCODER_FRAMES, EXCLUDE_OVERLAP,
    FBANK_FRAMES, MEL_BINS, MIN_SEGMENT_SECONDS, PLDA_DIM, SAMPLE_RATE, SEGMENTATION_FRAMES,
    WINDOW_SAMPLES, WINDOW_SECONDS,
};
use super::fbank::Fbank;
use super::plda::Plda;
use super::types::{SegChunk, StageError, TimedEmbedding};

/// `OfflineEmbeddingExtractor.swift` line 314: a speaker counts as active in
/// a frame above this weight.
const OVERLAP_THRESHOLD: f32 = 1e-3;
/// Line 527: a speaker needs at least this share of the window's frames, in
/// summed weight, after the overlap frames are taken out.
const MIN_ACTIVE_RATIO: f32 = 0.2;
/// `embedding.mil` `op_600` and `op_615` (fp16 0x1.a38p-14, the nearest to
/// 1e-4): added to `v1` and to the variance denominator.
const POOLING_EPS: f32 = 1e-4;
/// `embedding.mil` `op_34` (fp16 0x1.1p-20, 17 x 2^-24 = 1.013e-6, the nearest
/// to 1e-6): the floor of the variance before the square root.
const STD_FLOOR: f32 = 1e-6;
/// `embedding.mil` `op_644` (fp16 0x1.a38p-14, 1e-4): the floor of the norm
/// the embedding is divided by.
const NORM_FLOOR: f32 = 1e-4;
/// Mean and standard deviation of each of the 2560 channels.
const STATS_DIM: usize = 2 * ENCODER_CHANNELS;

/// The ResNet speaker encoder (`embedding_encoder.onnx`).
pub trait Encoder {
    /// `fbank` holds `batch` windows of `FBANK_FRAMES x MEL_BINS` values (the
    /// ONNX input `[B, 998, 80]`). The answer holds `batch x ENCODER_CHANNELS x
    /// ENCODER_FRAMES` values, row major (`[B, 2560, 125]`).
    fn encode(&mut self, fbank: &[f32], batch: usize) -> Result<Vec<f32>, StageError>;
}

/// The last linear layer of the embedding model (`resnet_seg_1`): 5120
/// statistics to 256 values.
pub struct Head {
    /// `[EMBEDDING_DIM, STATS_DIM]`, row major.
    weight: Vec<f32>,
    bias: Vec<f32>,
}

impl Head {
    /// Reads `resnet_seg_1_weight.npy` (shape `(256, 5120)`) and
    /// `resnet_seg_1_bias.npy` (shape `(256,)`): `.npy` version 1, little-endian
    /// f32, C order.
    pub fn from_npy(weight: &[u8], bias: &[u8]) -> Result<Head, &'static str> {
        Ok(Head {
            weight: parse_npy_f32(weight, &[EMBEDDING_DIM, STATS_DIM])?,
            bias: parse_npy_f32(bias, &[EMBEDDING_DIM])?,
        })
    }

    /// `weight . stats + bias`.
    fn apply(&self, stats: &[f32]) -> Vec<f32> {
        self.weight
            .as_chunks::<STATS_DIM>()
            .0
            .iter()
            .zip(&self.bias)
            .map(|(row, &bias)| dot(row, stats) + bias)
            .collect()
    }
}

/// Reads an `.npy` version 1 file of little-endian f32 in C order with exactly
/// this shape.
fn parse_npy_f32(bytes: &[u8], shape: &[usize]) -> Result<Vec<f32>, &'static str> {
    const MAGIC: &[u8] = b"\x93NUMPY";
    if bytes.len() < 10 || &bytes[..6] != MAGIC {
        return Err("npy: not a .npy file");
    }
    if bytes[6] != 1 {
        return Err("npy: only version 1 is supported");
    }
    let header_len: usize = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let header_bytes: &[u8] = bytes
        .get(10..10 + header_len)
        .ok_or("npy: truncated header")?;
    let header: &str = std::str::from_utf8(header_bytes).map_err(|_| "npy: header is not text")?;

    let descr: &str = header_value(header, "'descr'").ok_or("npy: no dtype")?;
    if !descr.starts_with("'<f4'") && !descr.starts_with("\"<f4\"") {
        return Err("npy: dtype is not little-endian f32");
    }
    let order: &str = header_value(header, "'fortran_order'").ok_or("npy: no order")?;
    if !order.starts_with("False") {
        return Err("npy: array is in Fortran order");
    }
    let shape_text: &str = header_value(header, "'shape'").ok_or("npy: no shape")?;
    let inner: &str = shape_text
        .strip_prefix('(')
        .and_then(|rest| rest.split(')').next())
        .ok_or("npy: bad shape")?;
    let mut found: Vec<usize> = Vec::new();
    for part in inner.split(',') {
        let part: &str = part.trim();
        if part.is_empty() {
            continue;
        }
        found.push(part.parse::<usize>().map_err(|_| "npy: bad shape")?);
    }
    if found != shape {
        return Err("npy: wrong shape");
    }

    let data: &[u8] = &bytes[10 + header_len..];
    let count: usize = shape.iter().product();
    if data.len() != count * 4 {
        return Err("npy: data has the wrong length");
    }
    Ok(data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect())
}

/// The text after `key:` in a Python dict literal, leading blanks removed.
fn header_value<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    let after: &str = &header[header.find(key)? + key.len()..];
    let after: &str = after.trim_start().strip_prefix(':')?;
    Some(after.trim_start())
}

/// One output frame of the 589 to 125 resampling: the two input frames it
/// reads and the weight of the second.
#[derive(Clone, Copy)]
struct Tap {
    low: usize,
    high: usize,
    fraction: f32,
}

/// `embedding.mil` `const_0_to_fp16` (`[125, 1, 589]`) is a convolution by a
/// matrix that resamples the mask linearly with align-corners: output frame
/// `i` reads input position `i * 588 / 124`, so the first output is input frame
/// 0 and the last is input frame 588.
fn resampling_taps() -> Vec<Tap> {
    let last_input: usize = SEGMENTATION_FRAMES - 1;
    (0..ENCODER_FRAMES)
        .map(|index| {
            let position: f64 = (index * last_input) as f64 / (ENCODER_FRAMES - 1) as f64;
            let low: usize = (position.floor() as usize).min(last_input);
            Tap {
                low,
                high: (low + 1).min(last_input),
                fraction: (position - low as f64) as f32,
            }
        })
        .collect()
}

fn resample_mask(mask: &[f32], taps: &[Tap]) -> Vec<f32> {
    taps.iter()
        .map(|tap| mask[tap.low] * (1.0 - tap.fraction) + mask[tap.high] * tap.fraction)
        .collect()
}

/// Sum of products with eight independent accumulators, so the loop can use
/// SIMD without `unsafe`.
fn dot(left: &[f32], right: &[f32]) -> f32 {
    let mut lanes: [f32; 8] = [0.0; 8];
    let (left_blocks, left_tail) = left.as_chunks::<8>();
    let (right_blocks, right_tail) = right.as_chunks::<8>();
    for (a, b) in left_blocks.iter().zip(right_blocks) {
        for ((lane, &x), &y) in lanes.iter_mut().zip(a).zip(b) {
            *lane += x * y;
        }
    }
    let tail: f32 = left_tail.iter().zip(right_tail).map(|(x, y)| x * y).sum();
    lanes.iter().sum::<f32>() + tail
}

/// `sum(weights * (values - mean)^2)`, with the same lane layout as `dot`.
fn weighted_square_deviation(values: &[f32], weights: &[f32], mean: f32) -> f32 {
    let mut lanes: [f32; 8] = [0.0; 8];
    let (value_blocks, value_tail) = values.as_chunks::<8>();
    let (weight_blocks, weight_tail) = weights.as_chunks::<8>();
    for (a, b) in value_blocks.iter().zip(weight_blocks) {
        for ((lane, &x), &w) in lanes.iter_mut().zip(a).zip(b) {
            let difference: f32 = x - mean;
            *lane += w * difference * difference;
        }
    }
    let tail: f32 = value_tail
        .iter()
        .zip(weight_tail)
        .map(|(x, w)| {
            let difference: f32 = x - mean;
            w * difference * difference
        })
        .sum();
    lanes.iter().sum::<f32>() + tail
}

/// Masked statistics pooling (`embedding.mil` lines 361 to 400). `features` is
/// `channels x frames` row major, `mask` has `frames` weights. The answer is
/// the weighted means of the channels, then their weighted standard
/// deviations: `2 * channels` values.
///
/// ```text
/// v1    = sum(w) + 1e-4
/// mean  = sum(x w) / v1
/// v2    = sum(w^2)
/// denom = v1 - v2 / v1 + 1e-4
/// std   = sqrt(max(sum(w (x - mean)^2) / denom, 1e-6))
/// ```
///
/// A frame is not dropped when its weight is small: it keeps its weight in
/// every sum, and `denom` is the unbiased-estimator correction for weights
/// (for 0 or 1 weights it is the number of active frames minus 1), so the
/// result differs from the plain standard deviation of the active frames.
fn masked_stats(features: &[f32], channels: usize, frames: usize, mask: &[f32]) -> Vec<f32> {
    let weight_sum: f32 = mask.iter().sum();
    let v1: f32 = weight_sum + POOLING_EPS;
    let v2: f32 = mask.iter().map(|weight| weight * weight).sum();
    let denominator: f32 = v1 - v2 / v1 + POOLING_EPS;

    let mut stats: Vec<f32> = vec![0.0; 2 * channels];
    for (channel, row) in features.chunks_exact(frames).take(channels).enumerate() {
        let mean: f32 = dot(row, mask) / v1;
        let variance: f32 = weighted_square_deviation(row, mask, mean) / denominator;
        stats[channel] = mean;
        stats[channels + channel] = variance.max(STD_FLOOR).sqrt();
    }
    stats
}

/// `x / max(|x|, 1e-4)` (`embedding.mil` `norms_1`, `clip_1`): a vector with
/// a norm under the floor is shrunk, not scaled up to unit length.
fn l2_normalise(values: &mut [f32]) {
    let norm: f32 = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    let divisor: f32 = norm.max(NORM_FLOOR);
    for value in values.iter_mut() {
        *value /= divisor;
    }
}

/// One local speaker of a window that has a usable mask.
struct SpeakerPlan {
    speaker_index: usize,
    /// `maskToUse` of the Swift: `SEGMENTATION_FRAMES` weights.
    mask: Vec<f32>,
}

/// What `processChunk` decides before any neural network runs.
struct ChunkPlan<'a> {
    chunk_index: usize,
    offset_seconds: f64,
    frame_duration: f64,
    audio: &'a [f32],
    speakers: Vec<SpeakerPlan>,
}

/// The metadata of an embedding waiting for its PLDA features.
struct Pending {
    chunk_index: usize,
    speaker_index: usize,
    start_frame: usize,
    end_frame: usize,
    frame_weights: Vec<f32>,
    start_time: f64,
    end_time: f64,
}

/// Seconds one segmentation frame covers (`resolveFrameDuration`, line 378).
fn frame_duration_of(chunk: &SegChunk) -> f64 {
    if chunk.frame_duration.is_finite() && chunk.frame_duration > 0.0 {
        chunk.frame_duration
    } else {
        WINDOW_SECONDS / chunk.speaker_weights.len().max(1) as f64
    }
}

/// `requiredMinFrames` (line 389): frames in `MIN_SEGMENT_SECONDS`, at least 1.
fn required_min_frames(frame_duration: f64) -> usize {
    if frame_duration <= 0.0 {
        return 1;
    }
    ((MIN_SEGMENT_SECONDS / frame_duration).ceil() as usize).max(1)
}

/// The first half of `processChunk` (lines 429 to 560) and the window
/// placement of the loop that feeds it (lines 659 to 690). `None` when the
/// window has nothing to embed.
fn plan_chunk<'a>(chunk: &SegChunk, track: &'a [f32]) -> Result<Option<ChunkPlan<'a>>, StageError> {
    let rows: &[Vec<f32>] = &chunk.speaker_weights;
    if rows.is_empty() {
        return Ok(None);
    }
    let frame_duration: f64 = frame_duration_of(chunk);
    let offset_seconds: f64 = if chunk.offset_seconds.is_finite() {
        chunk.offset_seconds
    } else {
        chunk.index as f64 * WINDOW_SECONDS
    };
    let start: usize = (offset_seconds * SAMPLE_RATE as f64)
        .round()
        .clamp(0.0, track.len() as f64) as usize;
    let end: usize = (start + WINDOW_SAMPLES).min(track.len());
    if start >= end {
        return Ok(None);
    }

    let frame_count: usize = rows.len();
    if frame_count != SEGMENTATION_FRAMES {
        return Err(StageError::Invalid(
            "a segmentation window does not have the expected number of frames",
        ));
    }
    let speaker_count: usize = rows[0].len();
    if speaker_count == 0 {
        return Ok(None);
    }
    if rows.iter().any(|row| row.len() != speaker_count) {
        return Err(StageError::Invalid(
            "a segmentation window has rows of different sizes",
        ));
    }

    let min_frames: f32 = required_min_frames(frame_duration) as f32;
    let overlap: Vec<bool> = rows
        .iter()
        .map(|row| {
            row.iter()
                .filter(|&&value| value > OVERLAP_THRESHOLD)
                .count()
                > 1
        })
        .collect();

    let mut speakers: Vec<SpeakerPlan> = Vec::new();
    for speaker_index in 0..speaker_count {
        let base: Vec<f32> = rows
            .iter()
            .map(|row| {
                let value: f32 = row[speaker_index];
                if value.is_finite() {
                    value
                } else {
                    0.0
                }
            })
            .collect();
        let base_sum: f32 = base.iter().sum();
        if base_sum <= 0.0 {
            continue;
        }
        let mut clean: Vec<f32> = base.clone();
        if EXCLUDE_OVERLAP {
            for (weight, &is_overlap) in clean.iter_mut().zip(&overlap) {
                if is_overlap {
                    *weight = 0.0;
                }
            }
        }
        let clean_sum: f32 = clean.iter().sum();
        if clean_sum < frame_count as f32 * MIN_ACTIVE_RATIO {
            continue;
        }
        let (mask, mask_sum): (Vec<f32>, f32) = if clean_sum >= min_frames {
            (clean, clean_sum)
        } else {
            (base, base_sum)
        };
        if mask_sum <= 0.0 {
            continue;
        }
        speakers.push(SpeakerPlan {
            speaker_index,
            mask,
        });
    }
    if speakers.is_empty() {
        return Ok(None);
    }
    Ok(Some(ChunkPlan {
        chunk_index: chunk.index,
        offset_seconds,
        frame_duration,
        audio: &track[start..end],
        speakers,
    }))
}

/// Turns segmentation into embeddings, a group of windows at a time.
pub struct Extractor {
    fbank: Fbank,
    plda: Plda,
    head: Head,
    taps: Vec<Tap>,
    embeddings: Vec<TimedEmbedding>,
}

impl Extractor {
    /// `plda` must take 256 values and give 128.
    pub fn new(plda: Plda, head: Head) -> Result<Extractor, &'static str> {
        if plda.input_dim() != EMBEDDING_DIM || plda.output_dim() != PLDA_DIM {
            return Err("embedding: the PLDA transform is not 256 to 128");
        }
        Ok(Extractor {
            fbank: Fbank::new(),
            plda,
            head,
            taps: resampling_taps(),
            embeddings: Vec::new(),
        })
    }

    /// Embeds the speakers of `chunks`, which are windows of `track` (the whole
    /// track, 16 kHz mono). Call it as often as segmentation produces windows;
    /// at most `EMBEDDING_BATCH` windows go through the encoder together.
    /// `stop` is read between windows and between speakers; once it answers
    /// true the call returns `StageError::Stopped` and the windows of that
    /// group are lost.
    pub fn process(
        &mut self,
        track: &[f32],
        chunks: &[SegChunk],
        encoder: &mut dyn Encoder,
        stop: &dyn Fn() -> bool,
    ) -> Result<(), StageError> {
        for group in chunks.chunks(EMBEDDING_BATCH) {
            if stop() {
                return Err(StageError::Stopped);
            }
            self.process_group(track, group, encoder, stop)?;
        }
        Ok(())
    }

    /// Everything embedded so far, in the order of the windows and, within a
    /// window, of the local speakers.
    pub fn finish(self) -> Vec<TimedEmbedding> {
        self.embeddings
    }

    fn process_group(
        &mut self,
        track: &[f32],
        group: &[SegChunk],
        encoder: &mut dyn Encoder,
        stop: &dyn Fn() -> bool,
    ) -> Result<(), StageError> {
        let mut plans: Vec<ChunkPlan<'_>> = Vec::with_capacity(group.len());
        for chunk in group {
            if let Some(plan) = plan_chunk(chunk, track)? {
                plans.push(plan);
            }
        }
        if plans.is_empty() {
            return Ok(());
        }

        let mut fbank: Vec<f32> = Vec::with_capacity(plans.len() * FBANK_FRAMES * MEL_BINS);
        for plan in &plans {
            if stop() {
                return Err(StageError::Stopped);
            }
            fbank.extend(self.fbank.window(plan.audio));
        }
        if stop() {
            return Err(StageError::Stopped);
        }
        let features: Vec<f32> = encoder.encode(&fbank, plans.len())?;
        drop(fbank);
        let per_window: usize = ENCODER_CHANNELS * ENCODER_FRAMES;
        if features.len() != plans.len() * per_window {
            return Err(StageError::Model(
                "the encoder returned an unexpected number of values".to_string(),
            ));
        }

        let mut pending: Vec<Pending> = Vec::new();
        let mut vectors: Vec<Vec<f32>> = Vec::new();
        for (plan, window_features) in plans.iter().zip(features.chunks_exact(per_window)) {
            for speaker in &plan.speakers {
                if stop() {
                    return Err(StageError::Stopped);
                }
                let weights: Vec<f32> = resample_mask(&speaker.mask, &self.taps);
                let stats: Vec<f32> =
                    masked_stats(window_features, ENCODER_CHANNELS, ENCODER_FRAMES, &weights);
                let mut embedding: Vec<f32> = self.head.apply(&stats);
                l2_normalise(&mut embedding);
                if embedding.iter().any(|value| !value.is_finite()) {
                    log::warn!(
                        "embedding of window {} speaker {} is not finite, skipped",
                        plan.chunk_index,
                        speaker.speaker_index
                    );
                    continue;
                }

                let first: usize = speaker
                    .mask
                    .iter()
                    .position(|&weight| weight > OVERLAP_THRESHOLD)
                    .unwrap_or(0);
                let last: usize = speaker
                    .mask
                    .iter()
                    .rposition(|&weight| weight > OVERLAP_THRESHOLD)
                    .unwrap_or(first);
                pending.push(Pending {
                    chunk_index: plan.chunk_index,
                    speaker_index: speaker.speaker_index,
                    start_frame: first,
                    end_frame: last,
                    frame_weights: speaker.mask.clone(),
                    start_time: plan.offset_seconds + first as f64 * plan.frame_duration,
                    end_time: plan.offset_seconds + (last + 1) as f64 * plan.frame_duration,
                });
                vectors.push(embedding);
            }
        }

        let rho: Vec<Vec<f64>> = self.plda.transform(&vectors);
        for ((meta, embedding256), rho128) in pending.into_iter().zip(vectors).zip(rho) {
            self.embeddings.push(TimedEmbedding {
                chunk_index: meta.chunk_index,
                speaker_index: meta.speaker_index,
                start_frame: meta.start_frame,
                end_frame: meta.end_frame,
                frame_weights: meta.frame_weights,
                start_time: meta.start_time,
                end_time: meta.end_time,
                embedding256,
                rho128,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
