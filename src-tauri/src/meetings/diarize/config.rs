//! The parameters of the pipeline, as named constants. They are the values of
//! `OfflineDiarizerConfig` in FluidAudio 0.17.7 (revision 503b4bd1, Apache-2.0),
//! `Sources/FluidAudio/Diarizer/Offline/Core/OfflineDiarizerTypes.swift`: the
//! `.community` statics that the macOS helper runs with
//! (`OfflineDiarizerManager()` uses the default configuration). Do not tune
//! them: they are what the Mac does.

/// `Segmentation.community`, line 46: audio is cut in windows of this length.
pub const WINDOW_SECONDS: f64 = 10.0;
/// `Segmentation.community`, line 46: the sample rate of the whole pipeline.
pub const SAMPLE_RATE: usize = 16_000;
/// Samples in one window (`samplesPerWindow`, line 348).
pub const WINDOW_SAMPLES: usize = 160_000;
/// Samples between two windows (`samplesPerStep`, line 352): 2 s, a window
/// every 20 % of its length (`stepRatio`, line 52).
pub const STEP_SAMPLES: usize = 32_000;
/// Frames the segmentation model gives for one window (Core ML and ONNX).
pub const SEGMENTATION_FRAMES: usize = 589;
/// Local speakers of one window. The powerset model has 7 classes over them.
pub const LOCAL_SPEAKERS: usize = 3;
/// Classes of the powerset output of the segmentation model.
pub const POWERSET_CLASSES: usize = 7;
/// Frames of 80 mel bins the fbank gives for one window (`1 + (160000 - 400) / 160`).
pub const FBANK_FRAMES: usize = 998;
/// Mel bins of the fbank.
pub const MEL_BINS: usize = 80;
/// Frames of the encoder output for one window.
pub const ENCODER_FRAMES: usize = 125;
/// Channels of the encoder output (256 channels x 10 frequency rows).
pub const ENCODER_CHANNELS: usize = 2560;
/// Size of one speaker embedding.
pub const EMBEDDING_DIM: usize = 256;
/// Size of an embedding after the PLDA transform.
pub const PLDA_DIM: usize = 128;

/// `Embedding.community`, line 100: windows the embedding model takes at once.
pub const EMBEDDING_BATCH: usize = 32;
/// `Embedding.community`: frames where two local speakers speak are left out
/// of the mask of each.
pub const EXCLUDE_OVERLAP: bool = true;
/// `Embedding.community`: the shortest stretch a speaker's mask may have to
/// be used with overlapped frames left out (`requiredMinFrames`, line 389),
/// and the shortest segment of the output (`OfflineReconstruction`). The gate
/// that decides whether a speaker gets an embedding at all is 20 % of the
/// window active (`OfflineEmbeddingExtractor.swift` line 527).
pub const MIN_SEGMENT_SECONDS: f64 = 1.0;

/// `Clustering.community`, line 155: the AHC cut distance.
pub const CLUSTERING_THRESHOLD: f64 = 0.6;
/// `Clustering.community`: VBx warm start, precision.
pub const VBX_FA: f64 = 0.07;
/// `Clustering.community`: VBx warm start, recall.
pub const VBX_FB: f64 = 0.8;
/// `Clustering.community`: two local speakers of one window never share a
/// cluster.
pub const CONSTRAINED_ASSIGNMENT: bool = true;

/// `VBx.community`, line 189.
pub const VBX_MAX_ITERATIONS: usize = 20;
/// `VBx.community`.
pub const VBX_CONVERGENCE_TOLERANCE: f64 = 1e-4;

/// `PostProcessing.community`, line 212: gaps shorter than this between two
/// segments of one speaker are closed.
pub const MIN_GAP_SECONDS: f64 = 0.1;
/// `PostProcessing.community`: one speaker at a time in the output.
pub const EXCLUSIVE_SEGMENTS: bool = true;
