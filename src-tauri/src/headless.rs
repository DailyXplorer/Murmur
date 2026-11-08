use super::*;

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

/// Headless one-shot transcription for `--transcribe-file`.
/// Drives the same `TranscriptionManager::transcribe` path as the desktop app.
/// Returns a process exit code (0 ok, 1 runtime failure, 2 bad input/usage).
pub(crate) fn run_headless_transcription(app: &AppHandle, args: &CliArgs) -> i32 {
    use std::time::Instant;

    let Some(wav) = args.transcribe_file.clone() else {
        return 0;
    };

    // read_wav_samples decodes every file as 16-bit int samples without checking
    // the format. The app only saves 16 kHz mono 16-bit PCM, so reject anything
    // else instead of transcribing a mis-decoded or mis-timed signal.
    match hound::WavReader::open(&wav) {
        Ok(reader) => {
            let spec = reader.spec();
            if spec.sample_rate != 16_000
                || spec.channels != 1
                || spec.bits_per_sample != 16
                || spec.sample_format != hound::SampleFormat::Int
            {
                eprintln!(
                    "error: expected 16 kHz mono 16-bit PCM WAV, got {} Hz / {} ch / {}-bit {:?}",
                    spec.sample_rate, spec.channels, spec.bits_per_sample, spec.sample_format
                );
                return 2;
            }
        }
        Err(e) => {
            eprintln!("error: cannot open {}: {}", wav.display(), e);
            return 2;
        }
    }

    let samples = match crate::audio_toolkit::read_wav_samples(&wav) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to read {}: {}", wav.display(), e);
            return 2;
        }
    };
    let audio_secs = samples.len() as f64 / 16_000.0;
    let tm = app.state::<Arc<TranscriptionManager>>();
    let started = Instant::now();
    let text = match tm.transcribe_sync(samples) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: transcribe failed: {error}");
            return 1;
        }
    };
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
                "backend": "chatgpt-session",
                "audio_secs": audio_secs,
                "transcribe_ms": elapsed_ms,
                "rtf": rtf,
                "text": text,
            })
        );
    } else {
        println!(
            "backend=chatgpt-session audio={audio_secs:.2}s transcribe={elapsed_ms}ms rtf={rtf:.2}x"
        );
        println!("text: {}", text);
    }
    0
}
