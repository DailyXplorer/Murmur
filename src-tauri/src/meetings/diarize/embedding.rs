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
mod tests {
    use super::super::plda::tests::keep_first_128;
    use super::*;
    use std::cell::Cell;

    // --- builders -----------------------------------------------------------

    /// A `.npy` version 1 file, laid out like numpy writes it (the header ends
    /// with a newline and the data starts on a multiple of 64).
    fn npy_bytes(descr: &str, fortran: &str, shape: &str, data: &[f32]) -> Vec<u8> {
        let mut header =
            format!("{{'descr': '{descr}', 'fortran_order': {fortran}, 'shape': {shape}, }}");
        while !(10 + header.len() + 1).is_multiple_of(64) {
            header.push(' ');
        }
        header.push('\n');
        let mut bytes: Vec<u8> = b"\x93NUMPY".to_vec();
        bytes.extend([1u8, 0u8]);
        bytes.extend((header.len() as u16).to_le_bytes());
        bytes.extend(header.as_bytes());
        bytes.extend(data.iter().flat_map(|value| value.to_le_bytes()));
        bytes
    }

    /// A head whose output is the 256 channel means, unchanged: row `o` has a
    /// 1 at column `o`.
    fn selector_head() -> Head {
        let mut weight: Vec<f32> = vec![0.0; EMBEDDING_DIM * STATS_DIM];
        for output in 0..EMBEDDING_DIM {
            weight[output * STATS_DIM + output] = 1.0;
        }
        let weight_bytes = npy_bytes("<f4", "False", "(256, 5120)", &weight);
        let bias_bytes = npy_bytes("<f4", "False", "(256,)", &vec![0.0; EMBEDDING_DIM]);
        Head::from_npy(&weight_bytes, &bias_bytes).expect("selector head loads")
    }

    fn new_extractor() -> Extractor {
        Extractor::new(keep_first_128(), selector_head()).expect("dimensions match")
    }

    /// A window of 589 frames and 3 speakers, each `(speaker, frames, weight)`
    /// of `active` filled in.
    fn chunk(
        index: usize,
        offset: f64,
        active: &[(usize, std::ops::Range<usize>, f32)],
    ) -> SegChunk {
        let mut weights: Vec<Vec<f32>> = vec![vec![0.0; 3]; SEGMENTATION_FRAMES];
        for (speaker, frames, value) in active {
            for frame in frames.clone() {
                weights[frame][*speaker] = *value;
            }
        }
        SegChunk {
            index,
            offset_seconds: offset,
            frame_duration: 10.0 / SEGMENTATION_FRAMES as f64,
            log_probs: Vec::new(),
            speaker_weights: weights,
        }
    }

    type Feature = Box<dyn Fn(usize, usize, usize) -> f32>;

    /// Records what it is given and answers `feature(window, channel, frame)`.
    struct FakeEncoder {
        batches: Vec<usize>,
        inputs: Vec<Vec<f32>>,
        feature: Feature,
        outcome: Option<StageError>,
        short_by: usize,
    }

    impl FakeEncoder {
        /// Channel `c` is `c + 1` in every frame and window.
        fn ramp() -> FakeEncoder {
            FakeEncoder::with(Box::new(|_, channel, _| channel as f32 + 1.0))
        }

        fn with(feature: Feature) -> FakeEncoder {
            FakeEncoder {
                batches: Vec::new(),
                inputs: Vec::new(),
                feature,
                outcome: None,
                short_by: 0,
            }
        }
    }

    impl Encoder for FakeEncoder {
        fn encode(&mut self, fbank: &[f32], batch: usize) -> Result<Vec<f32>, StageError> {
            assert_eq!(fbank.len(), batch * FBANK_FRAMES * MEL_BINS);
            self.batches.push(batch);
            self.inputs.push(fbank.to_vec());
            if let Some(error) = self.outcome.clone() {
                return Err(error);
            }
            let mut out: Vec<f32> = Vec::with_capacity(batch * ENCODER_CHANNELS * ENCODER_FRAMES);
            for window in 0..batch {
                for channel in 0..ENCODER_CHANNELS {
                    for frame in 0..ENCODER_FRAMES {
                        out.push((self.feature)(window, channel, frame));
                    }
                }
            }
            out.truncate(out.len() - self.short_by);
            Ok(out)
        }
    }

    fn never() -> bool {
        false
    }

    fn silence(seconds: usize) -> Vec<f32> {
        vec![0.0; seconds * SAMPLE_RATE]
    }

    fn run(chunks: &[SegChunk]) -> (Vec<TimedEmbedding>, FakeEncoder) {
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::ramp();
        extractor
            .process(&silence(30), chunks, &mut encoder, &never)
            .expect("process");
        (extractor.finish(), encoder)
    }

    // --- pooling ------------------------------------------------------------

    #[test]
    fn pooling_all_ones_uses_the_unbiased_denominator() {
        // Channel 0 = [1, 2, 3, 4, 5], all weights 1: mean 3, sum of squares
        // 10. With the +/-1e-4 terms the denominator is 5 - 1 = 4 (the number
        // of active frames minus one), so the variance is 2.5 and the standard
        // deviation 1.5811, not the population value sqrt(2) = 1.4142.
        let features: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 2.0, 2.0, 2.0, 2.0, 2.0];
        let stats = masked_stats(&features, 2, 5, &[1.0; 5]);
        assert_eq!(stats.len(), 4);
        assert!((stats[0] - 3.0).abs() < 1e-3, "{}", stats[0]);
        assert!((stats[2] - 1.5811).abs() < 1e-3, "{}", stats[2]);
        assert!(
            (stats[2] - 2f32.sqrt()).abs() > 0.1,
            "population std would pass"
        );
        // Channel 1 is constant: the variance floor gives sqrt(1e-6) = 1e-3.
        assert!((stats[1] - 2.0).abs() < 1e-3);
        assert!((stats[3] - 1e-3).abs() < 1e-4, "{}", stats[3]);
    }

    #[test]
    fn pooling_with_fractional_weights_matches_the_formula() {
        // w = [1, .5, 0, .5, 1], channel 0 = [1, 2, 3, 4, 5], channel 1 has a
        // huge value where the weight is 0.
        // sum w = 3, sum w x = 1 + 1 + 0 + 2 + 5 = 9, mean = 9 / 3 = 3
        // (3.0001 in the graph: 9 / 3.0001 = 2.99990).
        // v2 = 1 + .25 + 0 + .25 + 1 = 2.5, denominator = 3 - 2.5 / 3 = 2.16667
        // sum w (x - 3)^2 = 4 + .5 + 0 + .5 + 4 = 9, variance = 4.15385,
        // standard deviation 2.0381.
        let features: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 2.0, 2.0, 1000.0, 2.0, 2.0];
        let weights: [f32; 5] = [1.0, 0.5, 0.0, 0.5, 1.0];
        let stats = masked_stats(&features, 2, 5, &weights);
        assert!((stats[0] - 3.0).abs() < 1e-3, "{}", stats[0]);
        assert!((stats[2] - 2.0381).abs() < 2e-3, "{}", stats[2]);

        // The same with the exact constants of the graph, in f64.
        let v1 = 3.0 + 1e-4;
        let mean = 9.0 / v1;
        let denominator = v1 - 2.5 / v1 + 1e-4;
        let numerator: f64 = [1.0f64, 2.0, 3.0, 4.0, 5.0]
            .iter()
            .zip(weights)
            .map(|(x, w)| f64::from(w) * (x - mean) * (x - mean))
            .sum();
        let want_std = (numerator / denominator).sqrt();
        assert!((f64::from(stats[2]) - want_std).abs() < 1e-5);
        assert!((f64::from(stats[0]) - mean).abs() < 1e-5);

        // Dropping the zero-weight frame and using a plain weighted deviation
        // (denominator sum w = 3) gives sqrt(9 / 3) = 1.7321: a different
        // number, so the denominator is what the graph says.
        assert!((stats[2] - 3f32.sqrt()).abs() > 0.2);

        // The frame with weight 0 does not reach the statistics of channel 1.
        assert!((stats[1] - 2.0).abs() < 1e-3);
        assert!((stats[3] - 1e-3).abs() < 1e-4, "{}", stats[3]);
    }

    #[test]
    fn pooling_an_empty_mask_is_finite() {
        let stats = masked_stats(&[1.0, 2.0, 3.0], 1, 3, &[0.0; 3]);
        assert_eq!(stats[0], 0.0);
        assert!((stats[1] - 1e-3).abs() < 1e-4);
    }

    // --- resampling ---------------------------------------------------------

    #[test]
    fn resampling_is_linear_with_align_corners() {
        // A ramp m[k] = k resampled linearly reads back its own positions:
        // output i = i * 588 / 124 = 4.7419355 i.
        let taps = resampling_taps();
        assert_eq!(taps.len(), 125);
        let ramp: Vec<f32> = (0..589).map(|k| k as f32).collect();
        let out = resample_mask(&ramp, &taps);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[124], 588.0);
        assert!((out[1] - 4.741_935_5).abs() < 1e-4);
        assert!((out[2] - 9.483_871).abs() < 1e-3);
        assert!((out[62] - 294.0).abs() < 1e-3);
        // A hand-picked middle row: output 1 reads positions 4 and 5 with
        // weights 1 - 0.7419355 and 0.7419355.
        let mut mask: Vec<f32> = vec![0.0; 589];
        mask[4] = 1.0;
        mask[5] = 3.0;
        let out = resample_mask(&mask, &taps);
        assert!((out[1] - (0.258_064_5 + 3.0 * 0.741_935_5)).abs() < 1e-5);
        // The last output is the last input frame and nothing else.
        let mut edge: Vec<f32> = vec![0.0; 589];
        edge[588] = 1.0;
        edge[587] = 5.0;
        assert_eq!(resample_mask(&edge, &taps)[124], 1.0);
    }

    /// IEEE half precision to f32.
    fn half_to_f32(bits: u16) -> f32 {
        let sign: f32 = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent: i32 = i32::from((bits >> 10) & 0x1f);
        let fraction: f32 = f32::from(bits & 0x3ff);
        if exponent == 0 {
            sign * fraction * 2f32.powi(-24)
        } else {
            sign * (1.0 + fraction / 1024.0) * 2f32.powi(exponent - 15)
        }
    }

    #[test]
    #[ignore = "needs emb_head.bin in MURMUR_PLDA_DIR"]
    fn resampling_matrix_matches_the_core_ml_blob() {
        // `emb_head.bin` (next to the PLDA files in MURMUR_PLDA_DIR) starts
        // with `const_0_to_fp16`, [125, 1, 589] half-precision values, whose
        // blob header is at offset 64 (`embedding.mil` line 14).
        let dir = std::env::var("MURMUR_PLDA_DIR").expect("set MURMUR_PLDA_DIR");
        let blob = std::fs::read(format!("{dir}/emb_head.bin")).expect("emb_head.bin");
        let start = 64 + 64;
        let matrix: Vec<f32> = blob[start..start + 125 * 589 * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| half_to_f32(u16::from_le_bytes(*c)))
            .collect();
        let taps = resampling_taps();
        let mut worst: f32 = 0.0;
        for (row, tap) in taps.iter().enumerate() {
            let mut mine: Vec<f32> = vec![0.0; 589];
            mine[tap.low] += 1.0 - tap.fraction;
            mine[tap.high] += tap.fraction;
            for column in 0..589 {
                worst = worst.max((mine[column] - matrix[row * 589 + column]).abs());
            }
        }
        assert!(worst < 1e-3, "resampling matrix differs by {worst}");
    }

    #[test]
    #[ignore = "needs the .npy files in MURMUR_HEAD_DIR"]
    fn the_shipped_head_files_load() {
        let dir = std::env::var("MURMUR_HEAD_DIR").expect("set MURMUR_HEAD_DIR");
        let weight = std::fs::read(format!("{dir}/resnet_seg_1_weight.npy")).expect("weight");
        let bias = std::fs::read(format!("{dir}/resnet_seg_1_bias.npy")).expect("bias");
        let head = Head::from_npy(&weight, &bias).expect("the shipped files load");
        let out = head.apply(&vec![0.5; STATS_DIM]);
        assert_eq!(out.len(), EMBEDDING_DIM);
        assert!(out.iter().all(|value| value.is_finite()));
    }

    #[test]
    #[ignore = "prints timings of the pooling and the head, run in release"]
    fn time_pooling_and_head() {
        let features: Vec<f32> = (0..ENCODER_CHANNELS * ENCODER_FRAMES)
            .map(|index| (index % 97) as f32 * 0.01)
            .collect();
        let mask: Vec<f32> = (0..ENCODER_FRAMES).map(|i| (i % 3) as f32 * 0.5).collect();
        let head = selector_head();
        let runs = 200;
        let started = std::time::Instant::now();
        let mut sink = 0.0f32;
        for _ in 0..runs {
            sink += masked_stats(&features, ENCODER_CHANNELS, ENCODER_FRAMES, &mask)[7];
        }
        let pooling = started.elapsed() / runs;
        let stats = vec![0.25f32; STATS_DIM];
        let started = std::time::Instant::now();
        for _ in 0..runs {
            sink += head.apply(&stats)[3];
        }
        let applying = started.elapsed() / runs;
        let audio: Vec<f32> = (0..WINDOW_SAMPLES)
            .map(|i| ((i * 7919) % 2001) as f32 / 10_000.0 - 0.1)
            .collect();
        let fbank = Fbank::new();
        let started = std::time::Instant::now();
        for _ in 0..10 {
            sink += fbank.window(&audio)[11];
        }
        let features_time = started.elapsed() / 10;
        println!(
            "pooling {pooling:?} head {applying:?} per embedding, fbank {features_time:?} per window ({sink})"
        );
    }

    // --- normalisation ------------------------------------------------------

    #[test]
    fn l2_normalise_gives_unit_length_and_keeps_zero_finite() {
        let mut vector = vec![3.0, 4.0];
        l2_normalise(&mut vector);
        assert!((vector[0] - 0.6).abs() < 1e-6 && (vector[1] - 0.8).abs() < 1e-6);

        let mut zero = vec![0.0; 4];
        l2_normalise(&mut zero);
        assert_eq!(zero, vec![0.0; 4]);

        // Under the 1e-4 floor the vector is divided by the floor: not unit.
        let mut tiny = vec![1e-5, 0.0];
        l2_normalise(&mut tiny);
        assert!((tiny[0] - 0.1).abs() < 1e-6);
    }

    // --- the head and .npy --------------------------------------------------

    #[test]
    fn head_is_weight_times_stats_plus_bias() {
        // W[o][o] = 2 and W[o][5119] = 1, bias 0.5: stats with 4 at index 3
        // and 10 at index 5119 give 0.5 + 10 + (8 if o == 3).
        let mut weight: Vec<f32> = vec![0.0; EMBEDDING_DIM * STATS_DIM];
        for output in 0..EMBEDDING_DIM {
            weight[output * STATS_DIM + output] = 2.0;
            weight[output * STATS_DIM + 5119] = 1.0;
        }
        let head = Head::from_npy(
            &npy_bytes("<f4", "False", "(256, 5120)", &weight),
            &npy_bytes("<f4", "False", "(256,)", &vec![0.5; 256]),
        )
        .expect("loads");
        let mut stats: Vec<f32> = vec![0.0; STATS_DIM];
        stats[3] = 4.0;
        stats[5119] = 10.0;
        let out = head.apply(&stats);
        assert_eq!(out.len(), 256);
        assert!((out[3] - 18.5).abs() < 1e-5);
        assert!((out[0] - 10.5).abs() < 1e-5);
        assert!((out[255] - 10.5).abs() < 1e-5);
    }

    #[test]
    fn npy_files_that_are_not_the_head_are_refused() {
        let good_w = vec![0.0f32; EMBEDDING_DIM * STATS_DIM];
        let good_b = vec![0.0f32; EMBEDDING_DIM];
        let weight = npy_bytes("<f4", "False", "(256, 5120)", &good_w);
        let bias = npy_bytes("<f4", "False", "(256,)", &good_b);
        assert!(Head::from_npy(&weight, &bias).is_ok());

        let wrong_dtype = npy_bytes("<f8", "False", "(256,)", &good_b);
        assert_eq!(
            Head::from_npy(&weight, &wrong_dtype).err(),
            Some("npy: dtype is not little-endian f32")
        );
        let big_endian = npy_bytes(">f4", "False", "(256,)", &good_b);
        assert!(Head::from_npy(&weight, &big_endian).is_err());
        let fortran = npy_bytes("<f4", "True", "(256, 5120)", &good_w);
        assert_eq!(
            Head::from_npy(&fortran, &bias).err(),
            Some("npy: array is in Fortran order")
        );
        let wrong_shape = npy_bytes("<f4", "False", "(5120, 256)", &good_w);
        assert_eq!(
            Head::from_npy(&wrong_shape, &bias).err(),
            Some("npy: wrong shape")
        );
        let bias_matrix = npy_bytes("<f4", "False", "(256, 1)", &good_b);
        assert!(Head::from_npy(&weight, &bias_matrix).is_err());

        // Truncated data, truncated header, trailing bytes, not a .npy file.
        assert_eq!(
            Head::from_npy(&weight[..weight.len() - 4], &bias).err(),
            Some("npy: data has the wrong length")
        );
        assert_eq!(
            Head::from_npy(&weight, &bias[..40]).err(),
            Some("npy: truncated header")
        );
        let mut trailing = bias.clone();
        trailing.extend([0u8; 4]);
        assert!(Head::from_npy(&weight, &trailing).is_err());
        assert_eq!(
            Head::from_npy(b"hello", &bias).err(),
            Some("npy: not a .npy file")
        );
        assert!(Head::from_npy(&[], &bias).is_err());
        let mut version_2 = bias.clone();
        version_2[6] = 2;
        assert_eq!(
            Head::from_npy(&weight, &version_2).err(),
            Some("npy: only version 1 is supported")
        );
    }

    // --- the extractor ------------------------------------------------------

    #[test]
    fn no_chunks_give_no_embeddings_and_no_encoder_call() {
        let (embeddings, encoder) = run(&[]);
        assert!(embeddings.is_empty());
        assert!(encoder.batches.is_empty());
    }

    #[test]
    fn extractor_needs_a_256_to_128_transform() {
        use super::super::plda::tests::tiny_plda;
        assert!(Extractor::new(tiny_plda(), selector_head()).is_err());
    }

    #[test]
    fn a_speaker_gets_the_fields_the_swift_computes() {
        // Window at 2.0 s, speaker 0 active on frames 10..260 (weight 1).
        // frameDuration = 10 / 589 = 0.016977929 s.
        // startTime = 2 + 10 * 0.016977929 = 2.16977929,
        // endTime = 2 + (259 + 1) * 0.016977929 = 6.41426146.
        let (embeddings, encoder) = run(&[chunk(7, 2.0, &[(0, 10..260, 1.0)])]);
        assert_eq!(encoder.batches, vec![1]);
        assert_eq!(embeddings.len(), 1);
        let e = &embeddings[0];
        assert_eq!((e.chunk_index, e.speaker_index), (7, 0));
        assert_eq!((e.start_frame, e.end_frame), (10, 259));
        assert!((e.start_time - 2.169_779_29).abs() < 1e-6);
        assert!((e.end_time - 6.414_261_46).abs() < 1e-6);
        assert_eq!(e.frame_weights.len(), 589);
        assert_eq!(e.frame_weights.iter().sum::<f32>(), 250.0);
        assert_eq!(e.embedding256.len(), 256);
        assert_eq!(e.rho128.len(), 128);
    }

    #[test]
    fn embedding_is_the_normalised_pooled_mean() {
        // Channel c is constant c + 1, so its mean is (c + 1) sum(w) /
        // (sum(w) + 1e-4) and the common factor drops out of the L2 norm:
        // e[c] = (c + 1) / sqrt(sum_{k=1..256} k^2) = (c + 1) / sqrt(5625216).
        let (embeddings, _) = run(&[chunk(0, 0.0, &[(1, 0..300, 1.0)])]);
        let e = &embeddings[0];
        let norm = 5_625_216f64.sqrt();
        for channel in [0usize, 1, 100, 255] {
            let want = (channel as f64 + 1.0) / norm;
            assert!(
                (f64::from(e.embedding256[channel]) - want).abs() < 1e-6,
                "channel {channel}"
            );
        }
        let length: f32 = e.embedding256.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 1e-5);

        // The stand-in PLDA keeps the first 128 coordinates and normalises
        // them to length sqrt(128): rho[j] = sqrt(128) (j + 1) / sqrt(707264),
        // 707264 = sum_{k=1..128} k^2.
        for column in [0usize, 5, 127] {
            let want = 128f64.sqrt() * (column as f64 + 1.0) / 707_264f64.sqrt();
            assert!((e.rho128[column] - want).abs() < 1e-5, "column {column}");
        }
    }

    #[test]
    fn two_speakers_in_one_window_share_one_encoder_pass() {
        let (embeddings, encoder) = run(&[chunk(0, 0.0, &[(0, 0..200, 1.0), (2, 300..560, 0.5)])]);
        assert_eq!(encoder.batches, vec![1]);
        let speakers: Vec<usize> = embeddings.iter().map(|e| e.speaker_index).collect();
        assert_eq!(speakers, vec![0, 2]);
        assert_eq!(embeddings[1].frame_weights[300], 0.5);
    }

    #[test]
    fn a_speaker_below_the_minimum_gets_nothing_and_the_minimum_gets_one() {
        // frameCount * 0.2 = 589 * 0.2 = 117.8 frames of summed weight
        // (OfflineEmbeddingExtractor.swift line 527): 117 frames are not
        // enough, 118 are.
        let (embeddings, _) = run(&[
            chunk(0, 0.0, &[(0, 0..117, 1.0)]),
            chunk(1, 0.0, &[(0, 0..118, 1.0)]),
        ]);
        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0].chunk_index, 1);
        assert_eq!(
            (embeddings[0].start_frame, embeddings[0].end_frame),
            (0, 117)
        );
    }

    #[test]
    fn soft_weights_count_by_their_sum() {
        // 236 frames of 0.5 sum to 118, enough; 234 frames sum to 117, not.
        let (embeddings, _) = run(&[
            chunk(0, 0.0, &[(0, 0..236, 0.5)]),
            chunk(1, 0.0, &[(0, 0..234, 0.5)]),
        ]);
        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0].chunk_index, 0);
    }

    #[test]
    fn overlap_frames_are_left_out_of_both_masks() {
        // Speaker 0 on 0..260, speaker 1 on 200..460: frames 200..260 have two
        // active speakers. Each mask loses them.
        let (embeddings, _) = run(&[chunk(0, 0.0, &[(0, 0..260, 1.0), (1, 200..460, 1.0)])]);
        assert_eq!(embeddings.len(), 2);
        let (first, second) = (&embeddings[0], &embeddings[1]);
        assert!(first.frame_weights[200..260].iter().all(|&w| w == 0.0));
        assert!(second.frame_weights[200..260].iter().all(|&w| w == 0.0));
        assert_eq!(first.frame_weights.iter().sum::<f32>(), 200.0);
        assert_eq!(second.frame_weights.iter().sum::<f32>(), 200.0);
        assert_eq!((first.start_frame, first.end_frame), (0, 199));
        assert_eq!((second.start_frame, second.end_frame), (260, 459));
    }

    #[test]
    fn overlap_can_push_a_speaker_under_the_minimum() {
        // Speaker 0 has 150 frames but 40 are shared: 110 clean frames are
        // under 117.8, so there is no embedding although the raw sum passes.
        let (embeddings, _) = run(&[chunk(0, 0.0, &[(0, 0..150, 1.0), (1, 110..400, 1.0)])]);
        let speakers: Vec<usize> = embeddings.iter().map(|e| e.speaker_index).collect();
        assert_eq!(speakers, vec![1]);
    }

    #[test]
    fn the_overlap_threshold_is_a_weight_above_one_thousandth() {
        // A second speaker at exactly 1e-3 is not active, so no frame overlaps.
        let (embeddings, _) = run(&[chunk(
            0,
            0.0,
            &[(0, 0..200, 1.0), (1, 0..200, 1e-3), (2, 300..500, 1.0)],
        )]);
        let first = &embeddings[0];
        assert_eq!(first.speaker_index, 0);
        assert_eq!(first.frame_weights.iter().sum::<f32>(), 200.0);
    }

    #[test]
    fn a_short_clean_mask_falls_back_to_the_whole_mask() {
        // With frames of 5 ms the minimum is ceil(1 s / 5 ms) = 200 frames.
        // Speaker 0 has 150 clean frames (passes the 117.8 test) but under 200,
        // so its mask is the raw one, overlap frames included (line 535).
        let mut window = chunk(0, 0.0, &[(0, 0..180, 1.0), (1, 150..300, 1.0)]);
        window.frame_duration = 0.005;
        let (embeddings, _) = run(&[window]);
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].frame_weights.iter().sum::<f32>(), 180.0);
        assert_eq!(embeddings[0].frame_weights[160], 1.0);
        assert_eq!(embeddings[1].frame_weights.iter().sum::<f32>(), 150.0);
        assert!((embeddings[0].end_time - 0.005 * 180.0).abs() < 1e-9);
    }

    #[test]
    fn windows_without_speech_are_not_encoded() {
        let (embeddings, encoder) = run(&[chunk(0, 0.0, &[]), chunk(1, 0.0, &[(0, 0..50, 1.0)])]);
        assert!(embeddings.is_empty());
        assert!(encoder.batches.is_empty());
    }

    #[test]
    fn unusable_windows_are_skipped_without_error() {
        let mut empty = chunk(0, 0.0, &[(0, 0..200, 1.0)]);
        empty.speaker_weights.clear();
        let mut no_speakers = chunk(1, 0.0, &[]);
        for row in &mut no_speakers.speaker_weights {
            row.clear();
        }
        // The track is 30 s long; a window at 100 s starts past its end.
        let beyond = chunk(2, 100.0, &[(0, 0..200, 1.0)]);
        let (embeddings, encoder) = run(&[empty, no_speakers, beyond]);
        assert!(embeddings.is_empty());
        assert!(encoder.batches.is_empty());
    }

    #[test]
    fn a_non_finite_offset_falls_back_to_the_window_index() {
        // Swift line 666: index * 10 s.
        let mut window = chunk(1, f64::NAN, &[(0, 0..200, 1.0)]);
        window.frame_duration = f64::NAN;
        let (embeddings, _) = run(&[window]);
        assert_eq!(embeddings.len(), 1);
        assert!((embeddings[0].start_time - 10.0).abs() < 1e-9);
        // frame duration falls back to 10 s / 589.
        assert!((embeddings[0].end_time - (10.0 + 200.0 * 10.0 / 589.0)).abs() < 1e-9);
    }

    #[test]
    fn non_finite_weights_count_as_silence() {
        let mut window = chunk(0, 0.0, &[(0, 0..300, 1.0)]);
        window.speaker_weights[5][0] = f32::NAN;
        window.speaker_weights[6][0] = f32::INFINITY;
        let (embeddings, _) = run(&[window]);
        assert_eq!(embeddings.len(), 1);
        let e = &embeddings[0];
        assert!(e.frame_weights.iter().all(|w| w.is_finite()));
        assert_eq!(e.frame_weights.iter().sum::<f32>(), 298.0);
    }

    #[test]
    fn a_window_with_the_wrong_number_of_frames_is_invalid() {
        let mut window = chunk(0, 0.0, &[(0, 0..300, 1.0)]);
        window.speaker_weights.truncate(500);
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::ramp();
        let result = extractor.process(&silence(30), &[window], &mut encoder, &never);
        assert!(matches!(result, Err(StageError::Invalid(_))));
        let mut ragged = chunk(0, 0.0, &[(0, 0..300, 1.0)]);
        ragged.speaker_weights[3].pop();
        let result = extractor.process(&silence(30), &[ragged], &mut encoder, &never);
        assert!(matches!(result, Err(StageError::Invalid(_))));
    }

    #[test]
    fn windows_go_to_the_encoder_in_batches_of_32_with_their_own_audio() {
        let chunks: Vec<SegChunk> = (0..70)
            .map(|i| chunk(i, 0.0, &[(0, 0..200, 1.0)]))
            .collect();
        let (embeddings, encoder) = run(&chunks);
        assert_eq!(encoder.batches, vec![32, 32, 6]);
        assert_eq!(embeddings.len(), 70);
        let order: Vec<usize> = embeddings.iter().map(|e| e.chunk_index).collect();
        assert_eq!(order, (0..70).collect::<Vec<usize>>());
    }

    #[test]
    fn the_encoder_gets_the_fbank_of_each_window_zero_padded_at_the_end() {
        // 12 s of noise: windows at 0 s and 2 s are full, the window at 4 s is
        // cut at 12 s (128000 of 160000 samples) and padded with zeros.
        let mut state: u64 = 99;
        let track: Vec<f32> = (0..12 * SAMPLE_RATE)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                ((state >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.2
            })
            .collect();
        let chunks = vec![
            chunk(0, 0.0, &[(0, 0..200, 1.0)]),
            chunk(1, 2.0, &[(0, 0..200, 1.0)]),
            chunk(2, 4.0, &[(0, 0..200, 1.0)]),
        ];
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::ramp();
        extractor
            .process(&track, &chunks, &mut encoder, &never)
            .expect("process");
        assert_eq!(encoder.batches, vec![3]);
        let fbank = Fbank::new();
        let size = FBANK_FRAMES * MEL_BINS;
        for (window, start) in [0usize, 32_000, 64_000].into_iter().enumerate() {
            let end = (start + WINDOW_SAMPLES).min(track.len());
            let want = fbank.window(&track[start..end]);
            assert_eq!(
                &encoder.inputs[0][window * size..(window + 1) * size],
                &want[..]
            );
        }
    }

    #[test]
    fn cancellation_returns_stopped() {
        let chunks = vec![chunk(0, 0.0, &[(0, 0..200, 1.0)])];
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::ramp();
        let result = extractor.process(&silence(30), &chunks, &mut encoder, &|| true);
        assert_eq!(result, Err(StageError::Stopped));
        assert!(encoder.batches.is_empty());
        assert!(extractor.finish().is_empty());

        // Stopping after the encoder ran, between two speakers.
        let two = vec![chunk(0, 0.0, &[(0, 0..200, 1.0), (1, 300..500, 1.0)])];
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::ramp();
        let calls = Cell::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() >= 5
        };
        let result = extractor.process(&silence(30), &two, &mut encoder, &stop);
        assert_eq!(result, Err(StageError::Stopped));
        assert_eq!(encoder.batches, vec![1]);
        assert!(extractor.finish().is_empty());
    }

    #[test]
    fn an_empty_chunk_list_does_not_ask_to_stop() {
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::ramp();
        assert_eq!(
            extractor.process(&silence(1), &[], &mut encoder, &|| true),
            Ok(())
        );
    }

    #[test]
    fn encoder_failures_come_back() {
        let chunks = vec![chunk(0, 0.0, &[(0, 0..200, 1.0)])];
        let mut extractor = new_extractor();
        let mut failing = FakeEncoder::ramp();
        failing.outcome = Some(StageError::Model("boom".to_string()));
        assert_eq!(
            extractor.process(&silence(30), &chunks, &mut failing, &never),
            Err(StageError::Model("boom".to_string()))
        );
        let mut short = FakeEncoder::ramp();
        short.short_by = 1;
        assert!(matches!(
            extractor.process(&silence(30), &chunks, &mut short, &never),
            Err(StageError::Model(_))
        ));
    }

    #[test]
    fn a_non_finite_encoder_output_gives_no_embedding() {
        let chunks = vec![chunk(0, 0.0, &[(0, 0..200, 1.0)])];
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::with(Box::new(
            |_, channel, _| {
                if channel == 3 {
                    f32::NAN
                } else {
                    1.0
                }
            },
        ));
        extractor
            .process(&silence(30), &chunks, &mut encoder, &never)
            .expect("process");
        assert!(extractor.finish().is_empty());
    }

    #[test]
    fn calls_accumulate_in_order() {
        let mut extractor = new_extractor();
        let mut encoder = FakeEncoder::ramp();
        extractor
            .process(
                &silence(30),
                &[chunk(0, 0.0, &[(0, 0..200, 1.0)])],
                &mut encoder,
                &never,
            )
            .expect("first");
        extractor
            .process(
                &silence(30),
                &[chunk(1, 2.0, &[(1, 0..200, 1.0)])],
                &mut encoder,
                &never,
            )
            .expect("second");
        let embeddings = extractor.finish();
        assert_eq!(
            embeddings
                .iter()
                .map(|e| (e.chunk_index, e.speaker_index))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 1)]
        );
    }
}
