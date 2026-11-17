use super::*;
use crate::audio_toolkit::constants::MAX_RECORDING_SECONDS;
use crate::engine::Engine;
use crate::meetings::import_audio::decode::{decode_to_wav, DecodeError};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Convert an unexpected panic on the headless worker into a normal CLI
/// failure. Without this guard the Tauri event loop remains alive after the
/// worker exits, leaving `--transcribe-file` hung indefinitely.
pub(crate) fn run_headless_guarded<F>(operation: F) -> i32
where
    F: FnOnce() -> i32,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(code) => code,
        Err(payload) => {
            let message = if let Some(message) = payload.downcast_ref::<&str>() {
                (*message).to_string()
            } else if let Some(message) = payload.downcast_ref::<String>() {
                message.clone()
            } else {
                "unknown panic".to_string()
            };
            eprintln!("error: headless transcription panicked: {message}");
            1
        }
    }
}

/// The longest audio `--transcribe-file` takes: the file goes down the
/// dictation path, so the dictation limit applies.
const MAX_FILE: Duration = Duration::from_secs(MAX_RECORDING_SECONDS);

/// A decoded copy in the temporary folder, deleted when dropped.
struct TempWav(PathBuf);

impl TempWav {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let name = format!("murmur-transcribe-{}-{nanos}.wav", std::process::id());
        Self(std::env::temp_dir().join(name))
    }
}

impl TempWav {
    /// Deletes the copy now. A copy that is already gone is deleted.
    fn remove(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.0) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

/// The fallback for every early return and for a panic.
impl Drop for TempWav {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

/// Whether `path` is the WAV a dictation is saved as: 16 kHz mono 16-bit PCM,
/// which `read_wav_samples` reads without converting anything.
fn is_dictation_wav(path: &Path) -> bool {
    hound::WavReader::open(path).is_ok_and(|reader| {
        let spec = reader.spec();
        spec.sample_rate == 16_000
            && spec.channels == 1
            && spec.bits_per_sample == 16
            && spec.sample_format == hound::SampleFormat::Int
    })
}

/// The audio of `path` as the 16 kHz mono samples of a dictation, at most
/// `max` long. A WAV the app itself saves is read as it is. Any other file the
/// Meetings import reads (a WAV of another rate, depth or channel count, MP3,
/// M4A, AIFF, CAF, FLAC) is decoded and resampled first. The error is the
/// message for the terminal.
pub(crate) fn file_samples(path: &Path, max: Duration) -> Result<Vec<f32>, String> {
    let name = path.display();
    let samples = if is_dictation_wav(path) {
        crate::audio_toolkit::read_wav_samples(path)
            .map_err(|error| format!("failed to read {name}: {error}"))?
    } else {
        decoded_samples(path, max)?
    };
    // The decoder refuses a longer file while it reads it; the dictation WAV
    // is only measured here.
    let longest = max.as_millis().saturating_mul(16);
    if samples.len() as u128 > longest {
        return Err(too_long(path, max));
    }
    Ok(samples)
}

fn too_long(path: &Path, max: Duration) -> String {
    format!(
        "cannot transcribe {}: the audio is longer than {} minutes",
        path.display(),
        max.as_secs() / 60
    )
}

/// Decodes `path` through a copy in the temporary folder, deleted before this
/// returns. A copy that cannot be deleted does not cost the transcript: it is
/// named in a warning, as it holds audio of the user's file.
fn decoded_samples(path: &Path, max: Duration) -> Result<Vec<f32>, String> {
    let (samples, warning) = decode_through(path, max, TempWav::new(), TempWav::remove)?;
    if let Some(warning) = warning {
        eprintln!("warning: {warning}");
    }
    Ok(samples)
}

/// The samples of `path`, decoded into `copy`, and what to tell the user when
/// `remove` could not delete that copy afterwards. `remove` is `TempWav::remove`
/// outside the tests.
fn decode_through(
    path: &Path,
    max: Duration,
    copy: TempWav,
    remove: impl FnOnce(&TempWav) -> std::io::Result<()>,
) -> Result<(Vec<f32>, Option<String>), String> {
    let name = path.display();
    let max_ms = u64::try_from(max.as_millis()).unwrap_or(u64::MAX);
    decode_to_wav(path, &copy.0, max_ms, &AtomicBool::new(false), |_| {}).map_err(|error| {
        match error {
            DecodeError::TooLong => too_long(path, max),
            other => format!("cannot transcribe {name}: {other}"),
        }
    })?;
    let samples = crate::audio_toolkit::read_wav_samples(&copy.0)
        .map_err(|error| format!("failed to read the decoded copy of {name}: {error}"))?;
    let warning = remove(&copy).err().map(|error| {
        format!(
            "could not delete the temporary copy {}: {error}",
            copy.0.display()
        )
    });
    Ok((samples, warning))
}

/// The engine that transcribed, as the output of `--transcribe-file` names
/// it. The cloud keeps the name scripts already read. A file with no samples
/// reaches no engine.
fn backend_name(engine: Option<Engine>) -> &'static str {
    match engine {
        Some(Engine::Local) => "local-model",
        Some(Engine::Cloud) => "chatgpt-session",
        None => "none",
    }
}

/// Headless one-shot transcription for `--transcribe-file`.
/// Drives the same `TranscriptionManager::transcribe_samples` path as the desktop
/// app, and names the engine that answered.
/// Returns a process exit code (0 ok, 1 runtime failure, 2 bad input/usage).
pub(crate) fn run_headless_transcription(app: &AppHandle, args: &CliArgs) -> i32 {
    use std::time::Instant;

    let Some(file) = args.transcribe_file.clone() else {
        return 0;
    };

    let samples = match file_samples(&file, MAX_FILE) {
        Ok(samples) => samples,
        Err(message) => {
            eprintln!("error: {message}");
            return 2;
        }
    };
    let audio_secs = samples.len() as f64 / 16_000.0;
    let heard = !samples.is_empty();
    let tm = app.state::<Arc<TranscriptionManager>>();
    let started = Instant::now();
    let transcript = match tm.transcribe_sync(samples) {
        Ok(transcript) => transcript,
        Err(error) => {
            eprintln!("error: transcribe failed: {error}");
            return 1;
        }
    };
    let backend = backend_name(heard.then_some(transcript.source.engine));
    let text = transcript.text;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let rtf = if elapsed_ms > 0 {
        audio_secs / (elapsed_ms as f64 / 1000.0)
    } else {
        0.0
    };

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "backend": backend,
                "audio_secs": audio_secs,
                "transcribe_ms": elapsed_ms,
                "rtf": rtf,
                "text": text,
            })
        );
    } else {
        println!(
            "backend={backend} audio={audio_secs:.2}s transcribe={elapsed_ms}ms rtf={rtf:.2}x"
        );
        println!("text: {}", text);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{backend_name, decode_through, file_samples, TempWav};
    use crate::engine::Engine;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    const LONG: Duration = Duration::from_secs(60);

    /// The 0.4 s tones of the Meetings import tests.
    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/meetings/import_audio/fixtures")
            .join(name)
    }

    fn write_wav(path: &Path, spec: hound::WavSpec, frames: usize) {
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for frame in 0..frames {
            for _ in 0..spec.channels {
                match spec.sample_format {
                    hound::SampleFormat::Int => writer.write_sample(frame as i16).unwrap(),
                    hound::SampleFormat::Float => writer.write_sample(0.25f32).unwrap(),
                }
            }
        }
        writer.finalize().unwrap();
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn the_output_names_the_engine_that_transcribed() {
        assert_eq!(backend_name(Some(Engine::Local)), "local-model");
        assert_eq!(backend_name(Some(Engine::Cloud)), "chatgpt-session");
        // A file with no samples is settled before any engine.
        assert_eq!(backend_name(None), "none");
    }

    #[test]
    fn a_wav_the_app_saves_is_read_sample_for_sample() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictation.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        write_wav(&path, spec, 1_000);

        let samples = file_samples(&path, LONG).unwrap();

        let expected: Vec<f32> = (0..1_000).map(|v| v as f32 / i16::MAX as f32).collect();
        assert_eq!(samples, expected);
    }

    #[test]
    fn every_other_format_becomes_16_khz_mono() {
        for name in [
            "tone-44k-stereo.mp3",
            "tone-24k-mono.flac",
            "tone-44k-stereo-aac.m4a",
            "tone-22k-mono-alac.m4a",
            "tone-8k-mono-alac.caf",
            "tone-8k-mono-pcm.aiff",
        ] {
            let samples = file_samples(&fixture(name), LONG).unwrap();
            // 0.4 s of tone; lossy codecs add a few milliseconds of padding.
            assert!(
                (6_000..=8_000).contains(&samples.len()),
                "{name}: {}",
                samples.len()
            );
            assert!(rms(&samples) > 0.05, "{name} decoded to silence");
        }
    }

    #[test]
    fn a_wav_of_another_rate_and_layout_is_converted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        write_wav(&path, spec, 48_000);

        let samples = file_samples(&path, LONG).unwrap();

        assert_eq!(samples.len(), 16_000);
    }

    #[test]
    fn audio_over_the_limit_is_refused_with_the_limit_named() {
        let error =
            file_samples(&fixture("tone-24k-mono.flac"), Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("is longer than"), "{error}");
    }

    #[test]
    fn what_is_not_audio_and_what_is_missing_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("notes.wav");
        std::fs::write(&text, "not audio at all, only words").unwrap();
        let error = file_samples(&text, LONG).unwrap_err();
        assert!(error.contains("not supported"), "{error}");

        let error = file_samples(&dir.path().join("missing.m4a"), LONG).unwrap_err();
        assert!(error.contains("cannot read the file"), "{error}");
    }

    #[test]
    fn a_dictation_wav_over_the_limit_is_refused_like_any_other_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictation.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        // One second.
        write_wav(&path, spec, 16_000);

        let error = file_samples(&path, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("is longer than"), "{error}");
        assert_eq!(
            file_samples(&path, Duration::from_secs(1)).unwrap().len(),
            16_000
        );
    }

    #[test]
    fn a_copy_that_cannot_be_deleted_is_reported_and_one_already_gone_is_not() {
        let dir = tempfile::tempdir().unwrap();
        // `remove_file` refuses a folder.
        let stuck = TempWav(dir.path().to_path_buf());
        assert!(stuck.remove().is_err());

        let gone = TempWav(dir.path().join("never-written.wav"));
        assert!(gone.remove().is_ok());
    }

    #[test]
    fn a_copy_that_cannot_be_deleted_costs_a_warning_not_the_transcript() {
        let copy = TempWav::new();
        let kept = copy.0.clone();

        let (samples, warning) = decode_through(&fixture("tone-24k-mono.flac"), LONG, copy, |_| {
            Err(std::io::Error::other("the file is in use"))
        })
        .unwrap();

        assert!(
            (6_000..=8_000).contains(&samples.len()),
            "{}",
            samples.len()
        );
        let warning = warning.expect("the failed delete is reported");
        assert!(warning.contains(&kept.display().to_string()), "{warning}");
        assert!(warning.contains("the file is in use"), "{warning}");
        // The fallback still deleted it once the copy went out of scope.
        assert!(!kept.exists());

        let (_, warning) = decode_through(
            &fixture("tone-24k-mono.flac"),
            LONG,
            TempWav::new(),
            TempWav::remove,
        )
        .unwrap();
        assert_eq!(warning, None);
    }

    #[test]
    fn the_decoded_copy_is_deleted_when_dropped() {
        let copy = TempWav::new();
        let path = copy.0.clone();
        std::fs::write(&path, b"audio").unwrap();
        assert!(path.exists());
        drop(copy);
        assert!(!path.exists());
    }
}
