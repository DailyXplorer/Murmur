//! The data the stages of the pipeline hand to each other. They mirror
//! `SegmentationChunk`, `SegmentationOutput`, `TimedEmbedding` and
//! `DiarizationSegment` of FluidAudio 0.17.7.

/// What the segmentation of one window gives (`SegmentationChunk`,
/// `OfflineDiarizerTypes.swift` line 598).
#[derive(Debug, Clone, PartialEq)]
pub struct SegChunk {
    pub index: usize,
    /// Start of the window on the track, in seconds.
    pub offset_seconds: f64,
    /// Seconds one frame covers (`WINDOW_SECONDS / frames`).
    pub frame_duration: f64,
    /// Frames x classes, the log-probabilities of the powerset model. May be
    /// emptied once the embeddings of the window are made.
    pub log_probs: Vec<Vec<f32>>,
    /// Frames x local speakers, the soft activity of each, 0.0..=1.0.
    pub speaker_weights: Vec<Vec<f32>>,
}

/// Every window of a track, what reconstruction reads (`SegmentationOutput`,
/// line 567, without the log-probabilities).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Segmentation {
    /// `chunks[i].index == i`.
    pub chunks: Vec<SegChunk>,
    /// Frames per window.
    pub num_frames: usize,
    pub frame_duration: f64,
}

/// One embedding of one local speaker of one window (`TimedEmbedding`,
/// line 706).
#[derive(Debug, Clone, PartialEq)]
pub struct TimedEmbedding {
    pub chunk_index: usize,
    pub speaker_index: usize,
    pub start_frame: usize,
    pub end_frame: usize,
    pub frame_weights: Vec<f32>,
    pub start_time: f64,
    pub end_time: f64,
    /// L2-normalised, 256 values.
    pub embedding256: Vec<f32>,
    /// After the PLDA transform, 128 values.
    pub rho128: Vec<f64>,
}

/// A stretch of one speaker on the track, in seconds. `speaker` counts from 0
/// in the order FluidAudio names them (`S{speaker + 1}`).
#[derive(Debug, Clone, PartialEq)]
pub struct DiarSegment {
    pub speaker: usize,
    pub start: f64,
    pub end: f64,
}

/// Why a stage did not answer. Fixed words only, never audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageError {
    /// Cancelled, or past its time limit.
    Stopped,
    /// A model call failed.
    Model(String),
    /// A stage got input it cannot work with (a bug, never the user's audio).
    Invalid(&'static str),
}
