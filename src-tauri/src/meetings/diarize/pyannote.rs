//! Windows (x86_64): the two neural networks of the pipeline on ONNX Runtime,
//! and the `Diarizer` the meeting analysis runs (`analysis_cpp`).
//!
//! ONNX Runtime is the prebuilt CPU build the `ort` crate links statically:
//! no DLL ships beside the app and no `onnxruntime.dll` of the system can be
//! loaded by name. Only the CPU provider is used, with the arena allocator off
//! (it never gives memory back) and one window per encoder run, which is also
//! the fastest on a CPU. The prebuilt binaries need AVX2; a processor without
//! it never reaches ONNX Runtime (`Failure::Failed`, one speaker per track).
//!
//! Files that are missing or differ from the manifest are `Failure::Missing`,
//! so the analysis asks for the models again. Everything else that goes wrong
//! is `Failure::Failed` with a fixed word, never a path or a transcript.

use std::path::Path;
use std::sync::OnceLock;

use ort::environment::Environment;
use ort::ep;
use ort::session::Session;
use ort::value::Tensor;

use super::config::{
    ENCODER_CHANNELS, ENCODER_FRAMES, FBANK_FRAMES, MEL_BINS, POWERSET_CLASSES,
    SEGMENTATION_FRAMES, WINDOW_SAMPLES,
};
use super::embedding::{Encoder, Extractor, Head};
use super::plda::Plda;
use super::types::{DiarSegment, StageError};
use super::{diarize, SegmentationModel};
use crate::local_models::manifest::{
    pyannote_entry, pyannote_path, ENCODER_ONNX, HEAD_BIAS_NPY, HEAD_WEIGHT_NPY,
    PLDA_PARAMETERS_JSON, PYANNOTE_FILES, SEGMENTATION_ONNX, XVECTOR_TRANSFORM_JSON,
};
use crate::local_models::verify;
use crate::meetings::analysis_local::{Diarizer, Failure, Spoken};

/// Windows the segmentation network takes in one run.
const SEGMENTATION_BATCH: usize = 8;

fn model_error(_: ort::Error) -> StageError {
    StageError::Model("onnx run".to_string())
}

/// One environment for the process: ONNX Runtime allows one, and it holds no
/// memory a session does not.
fn environment() -> Result<&'static Environment, Failure> {
    static ENVIRONMENT: OnceLock<Result<Environment, &'static str>> = OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| {
            ort::init()
                .with_telemetry(false)
                .with_name("murmur")
                .build()
                .map_err(|_| "onnx environment")
        })
        .as_ref()
        .map_err(|why| Failure::Failed((*why).to_string()))
}

fn open(environment: &Environment, path: &Path, threads: usize) -> ort::Result<Session> {
    Session::builder(environment)?
        .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])?
        .with_memory_pattern(false)?
        .with_intra_threads(threads)?
        .with_inter_threads(1)?
        .commit_from_file(path)
}

/// `segmentation-community-1.onnx`.
struct Segmenter {
    session: Session,
}

impl SegmentationModel for Segmenter {
    fn segment(&mut self, windows: &[f32], count: usize) -> Result<Vec<f32>, StageError> {
        if windows.len() != count * WINDOW_SAMPLES {
            return Err(StageError::Invalid("segmentation input size"));
        }
        let mut logits: Vec<f32> =
            Vec::with_capacity(count * SEGMENTATION_FRAMES * POWERSET_CLASSES);
        for first in (0..count).step_by(SEGMENTATION_BATCH) {
            let batch = SEGMENTATION_BATCH.min(count - first);
            let input = Tensor::from_array((
                [batch, 1usize, WINDOW_SAMPLES],
                windows[first * WINDOW_SAMPLES..(first + batch) * WINDOW_SAMPLES].to_vec(),
            ))
            .map_err(model_error)?;
            let outputs = self
                .session
                .run(ort::inputs!["input_values" => input])
                .map_err(model_error)?;
            let (shape, values) = outputs["logits"]
                .try_extract_tensor::<f32>()
                .map_err(model_error)?;
            let dims: &[i64] = shape;
            if dims
                != [
                    batch as i64,
                    SEGMENTATION_FRAMES as i64,
                    POWERSET_CLASSES as i64,
                ]
            {
                return Err(StageError::Invalid("segmentation output shape"));
            }
            logits.extend_from_slice(values);
        }
        Ok(logits)
    }
}

/// `embedding_encoder.onnx`: the ResNet up to the statistics pooling.
struct Resnet {
    session: Session,
}

impl Encoder for Resnet {
    fn encode(&mut self, fbank: &[f32], batch: usize) -> Result<Vec<f32>, StageError> {
        let per_window = FBANK_FRAMES * MEL_BINS;
        if fbank.len() != batch * per_window {
            return Err(StageError::Invalid("encoder input size"));
        }
        let mut features: Vec<f32> = Vec::with_capacity(batch * ENCODER_CHANNELS * ENCODER_FRAMES);
        // One window per run: the same speed per window as a batch, and the
        // memory of one window.
        for window in fbank.chunks_exact(per_window) {
            let input = Tensor::from_array(([1usize, FBANK_FRAMES, MEL_BINS], window.to_vec()))
                .map_err(model_error)?;
            let outputs = self
                .session
                .run(ort::inputs!["fbank_features" => input])
                .map_err(model_error)?;
            let (shape, values) = outputs["/resnet/pool/Reshape_output_0"]
                .try_extract_tensor::<f32>()
                .map_err(model_error)?;
            let dims: &[i64] = shape;
            if dims != [1, ENCODER_CHANNELS as i64, ENCODER_FRAMES as i64] {
                return Err(StageError::Invalid("encoder output shape"));
            }
            features.extend_from_slice(values);
        }
        Ok(features)
    }
}

/// The files of the speaker model, read and checked, and the two sessions.
pub struct Pyannote<'a> {
    segmenter: Segmenter,
    encoder: Resnet,
    plda_parameters: String,
    xvector_transform: String,
    head_weight: Vec<u8>,
    head_bias: Vec<u8>,
    stop: &'a dyn Fn() -> bool,
}

fn read(dir: &Path, name: &str) -> Result<Vec<u8>, Failure> {
    let path = dir.join(pyannote_path(name));
    let entry = pyannote_entry(name).ok_or(Failure::Missing)?;
    match verify::matches(&path, entry) {
        Ok(true) => std::fs::read(&path).map_err(|_| Failure::Missing),
        Ok(false) | Err(_) => Err(Failure::Missing),
    }
}

fn text(bytes: Vec<u8>) -> Result<String, Failure> {
    String::from_utf8(bytes).map_err(|_| Failure::Missing)
}

/// Reads and checks the six files, then opens the sessions. `Missing` when a
/// file is absent or is not the pinned one; `Failed` when ONNX Runtime cannot
/// run here. `threads` is what the networks may use, `stop` is asked between
/// windows.
pub fn load<'a>(
    dir: &Path,
    threads: usize,
    stop: &'a dyn Fn() -> bool,
) -> Result<Pyannote<'a>, Failure> {
    debug_assert_eq!(PYANNOTE_FILES.len(), 6);
    let segmentation = dir.join(pyannote_path(SEGMENTATION_ONNX));
    let encoder = dir.join(pyannote_path(ENCODER_ONNX));
    // The networks are hashed through `read` as well, so a damaged one is
    // `Missing` and not an opaque error of ONNX Runtime.
    for name in [SEGMENTATION_ONNX, ENCODER_ONNX] {
        drop(read(dir, name)?);
    }
    let head_weight = read(dir, HEAD_WEIGHT_NPY)?;
    let head_bias = read(dir, HEAD_BIAS_NPY)?;
    let plda_parameters = text(read(dir, PLDA_PARAMETERS_JSON)?)?;
    let xvector_transform = text(read(dir, XVECTOR_TRANSFORM_JSON)?)?;
    // Files that are the pinned ones but do not parse are a build problem,
    // not a download problem, and say so.
    Head::from_npy(&head_weight, &head_bias).map_err(|why| Failure::Failed(why.to_string()))?;
    Plda::from_json(&plda_parameters, &xvector_transform)
        .map_err(|why| Failure::Failed(why.to_string()))?;
    if !std::is_x86_feature_detected!("avx2") {
        return Err(Failure::Failed("avx2".to_string()));
    }
    let environment = environment()?;
    let failed = |_: ort::Error| Failure::Failed("onnx session".to_string());
    let segmenter = Segmenter {
        session: open(environment, &segmentation, threads).map_err(failed)?,
    };
    let encoder = Resnet {
        session: open(environment, &encoder, threads).map_err(failed)?,
    };
    Ok(Pyannote {
        segmenter,
        encoder,
        plda_parameters,
        xvector_transform,
        head_weight,
        head_bias,
        stop,
    })
}

impl Pyannote<'_> {
    /// Segments of one speaker at a time, speakers numbered from 0 in the
    /// order they first speak.
    pub fn diarize(
        &mut self,
        track: &[f32],
        progress: &mut dyn FnMut(f64),
    ) -> Result<Vec<DiarSegment>, StageError> {
        let plda = Plda::from_json(&self.plda_parameters, &self.xvector_transform)
            .map_err(|_| StageError::Invalid("plda"))?;
        let psi = plda.psi().to_vec();
        let head = Head::from_npy(&self.head_weight, &self.head_bias)
            .map_err(|_| StageError::Invalid("head"))?;
        let extractor = Extractor::new(plda, head).map_err(StageError::Invalid)?;
        diarize(
            track,
            &mut self.segmenter,
            &mut self.encoder,
            extractor,
            &psi,
            self.stop,
            progress,
        )
    }
}

fn failure(error: StageError) -> Failure {
    match error {
        StageError::Stopped => Failure::Stopped,
        StageError::Model(what) => Failure::Failed(what),
        StageError::Invalid(why) => Failure::Failed(why.to_string()),
    }
}

/// The speakers of the pipeline as the analysis names them, `S1`, `S2`…
pub fn spoken(segments: &[DiarSegment]) -> Vec<Spoken> {
    segments
        .iter()
        .map(|segment| Spoken {
            speaker: i32::try_from(segment.speaker + 1).unwrap_or(i32::MAX),
            start_ms: (segment.start * 1000.0).round() as i64,
            end_ms: (segment.end * 1000.0).round() as i64,
        })
        .collect()
}

impl Diarizer for Pyannote<'_> {
    fn tell_apart(&mut self, track: &[f32]) -> Result<Vec<Spoken>, Failure> {
        self.tell_apart_reporting(track, &mut |_| {})
    }

    fn tell_apart_reporting(
        &mut self,
        track: &[f32],
        progress: &mut dyn FnMut(f64),
    ) -> Result<Vec<Spoken>, Failure> {
        self.diarize(track, progress)
            .map(|segments| spoken(&segments))
            .map_err(failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_models::manifest::PYANNOTE_FOLDER;

    fn nobody_stops() -> bool {
        false
    }

    /// A folder with every file of the speaker model, each with the pinned
    /// size but the wrong content.
    fn damaged_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(PYANNOTE_FOLDER)).unwrap();
        for name in PYANNOTE_FILES {
            let size = pyannote_entry(name).unwrap().size as usize;
            std::fs::write(dir.path().join(pyannote_path(name)), vec![7u8; size]).unwrap();
        }
        dir
    }

    #[test]
    fn a_models_folder_without_the_speaker_model_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load(dir.path(), 1, &nobody_stops).err(),
            Some(Failure::Missing)
        );
    }

    #[test]
    fn files_of_the_right_size_with_the_wrong_content_are_missing() {
        let dir = damaged_folder();
        assert_eq!(
            load(dir.path(), 1, &nobody_stops).err(),
            Some(Failure::Missing)
        );
    }

    #[test]
    fn a_file_of_the_wrong_size_is_missing() {
        let dir = damaged_folder();
        std::fs::write(dir.path().join(pyannote_path(HEAD_BIAS_NPY)), b"short").unwrap();
        assert_eq!(
            load(dir.path(), 1, &nobody_stops).err(),
            Some(Failure::Missing)
        );
    }

    #[test]
    fn segments_become_numbered_turns_in_milliseconds() {
        let segments = vec![
            DiarSegment {
                speaker: 0,
                start: 0.5,
                end: 2.25,
            },
            DiarSegment {
                speaker: 11,
                start: 3.0,
                end: 4.0004,
            },
        ];
        assert_eq!(
            spoken(&segments),
            [
                Spoken {
                    speaker: 1,
                    start_ms: 500,
                    end_ms: 2_250
                },
                Spoken {
                    speaker: 12,
                    start_ms: 3_000,
                    end_ms: 4_000
                },
            ]
        );
    }

    #[test]
    fn a_stopped_stage_is_a_stopped_run_and_other_errors_are_failures() {
        assert_eq!(failure(StageError::Stopped), Failure::Stopped);
        assert_eq!(
            failure(StageError::Model("onnx run".to_string())),
            Failure::Failed("onnx run".to_string())
        );
        assert_eq!(
            failure(StageError::Invalid("plda")),
            Failure::Failed("plda".to_string())
        );
    }
}
