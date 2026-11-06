//! Intel Macs: the GGUF model (8-bit) through transcribe.cpp, on the CPU in
//! the app. Forced to the CPU: Intel Macs have no GPU it can use.
//! Boost terms are not passed: transcribe-cpp 0.4.1's Parakeet does not set
//! `TRANSCRIBE_FEATURE_VOCABULARY`, so its `RunOptions.vocabulary` is dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use transcribe_cpp::{
    Backend, CancelToken, ErrorKind, Model, ModelOptions, RunOptions, Session, TimestampKind,
};

use super::session::{BoxFuture, Loaded, Runtime};
use crate::meetings::protocol::Word;

pub struct CpuRuntime {
    /// The models folder: which GGUF file it runs is read at each load.
    pub models: PathBuf,
}

impl Runtime for CpuRuntime {
    fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
        let models = self.models.clone();
        Box::pin(async move {
            let model = tokio::task::spawn_blocking(move || -> anyhow::Result<CpuModel> {
                let model = load_model(&models)?;
                let mut session = model.session()?;
                let cancel = CancelToken::new();
                session.set_cancel_token(&cancel);
                Ok(CpuModel {
                    session: Arc::new(Mutex::new(session)),
                    cancel,
                })
            })
            .await??;
            Ok(Box::new(model) as Box<dyn Loaded>)
        })
    }
}

/// The speech model the models folder names, loaded on the CPU. Blocking.
pub(crate) fn load_model(models: &Path) -> anyhow::Result<Model> {
    #[cfg(target_os = "windows")]
    load_cpu_modules()?;
    let options = ModelOptions {
        backend: Backend::Cpu,
        device: None,
        // Mapped from the file: the model is never rewritten in
        // place, only replaced by a rename.
        no_mmap: false,
    };
    let asr = crate::local_models::asr_in_use(models);
    Ok(
        match Model::load_with(models.join(asr.gguf_path()), &options) {
            // The switch to the current model deleted the previous
            // one between the choice and the open.
            Err(_) if crate::local_models::asr_in_use(models) != asr => {
                let asr = crate::local_models::asr_in_use(models);
                Model::load_with(models.join(asr.gguf_path()), &options)?
            }
            loaded => loaded?,
        },
    )
}

/// Windows ships the CPU code as one module per instruction set, beside the
/// app (see `build.rs`); transcribe.cpp loads the fastest this processor
/// runs. Once per process, before the first model load: it cannot be retried.
#[cfg(target_os = "windows")]
pub(crate) fn load_cpu_modules() -> anyhow::Result<()> {
    static LOADED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    LOADED
        .get_or_init(|| transcribe_cpp::init_backends_default().map_err(|error| error.to_string()))
        .clone()
        .map_err(|error| anyhow::anyhow!("Cannot load the CPU modules: {error}"))
}

struct CpuModel {
    /// Shared with the thread of a run, which may outlive an abandoned one.
    session: Arc<Mutex<Session>>,
    cancel: CancelToken,
}

/// Stops the run on its thread when the request is abandoned. `abandoned`
/// tells a run that has not taken the session yet, as the token is reset
/// when it does.
struct AbortOnDrop(Option<(CancelToken, Arc<AtomicBool>)>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some((cancel, abandoned)) = &self.0 {
            abandoned.store(true, Ordering::SeqCst);
            cancel.cancel();
        }
    }
}

/// Puts the calling thread below the normal priority until dropped. The
/// blocking pool reuses its threads, so the drop puts the old priority back.
#[cfg(target_os = "windows")]
struct BelowNormal {
    previous: i32,
}

#[cfg(target_os = "windows")]
impl BelowNormal {
    fn set() -> Self {
        use windows::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        // SAFETY: the pseudo handle of the calling thread needs no closing and
        // is used on this thread only.
        unsafe {
            let thread = GetCurrentThread();
            let previous = GetThreadPriority(thread);
            if SetThreadPriority(thread, THREAD_PRIORITY_BELOW_NORMAL).is_err() {
                log::debug!("Cannot lower the priority of a live meeting piece");
            }
            Self { previous }
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for BelowNormal {
    fn drop(&mut self) {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY,
        };
        // SAFETY: as in `set`, on the same thread.
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(self.previous)) };
    }
}

impl CpuModel {
    /// Runs `work` on the session in a blocking task. `lowered` runs it below
    /// the normal priority on Windows, for a piece no one waits on.
    fn run<T: Send + 'static>(
        &self,
        lowered: bool,
        work: impl FnOnce(&mut Session) -> anyhow::Result<T> + Send + 'static,
    ) -> BoxFuture<'static, anyhow::Result<T>> {
        let session = Arc::clone(&self.session);
        let cancel = self.cancel.clone();
        Box::pin(async move {
            let abandoned = Arc::new(AtomicBool::new(false));
            let mut abort = AbortOnDrop(Some((cancel.clone(), Arc::clone(&abandoned))));
            let result = tokio::task::spawn_blocking(move || -> anyhow::Result<T> {
                #[cfg(target_os = "windows")]
                let _priority = lowered.then(BelowNormal::set);
                #[cfg(not(target_os = "windows"))]
                let _ = lowered;
                let mut session = session
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                cancel.reset();
                // Read after the reset: a later abandon cancels the run.
                if abandoned.load(Ordering::SeqCst) {
                    anyhow::bail!("Local transcription abandoned");
                }
                work(&mut session)
            })
            .await??;
            abort.0 = None;
            Ok(result)
        })
    }
}

impl Loaded for CpuModel {
    fn transcribe<'a>(
        &'a mut self,
        samples: &'a [f32],
        _terms: &'a [String],
    ) -> BoxFuture<'a, anyhow::Result<String>> {
        let samples = samples.to_vec();
        self.run(false, move |session| {
            Ok(session.run(&samples, &RunOptions::default())?.text)
        })
    }

    fn transcribe_words<'a>(
        &'a mut self,
        samples: &'a [f32],
    ) -> BoxFuture<'a, anyhow::Result<Vec<Word>>> {
        let samples = samples.to_vec();
        self.run(true, move |session| {
            let options = RunOptions {
                timestamps: TimestampKind::Word,
                ..RunOptions::default()
            };
            let transcript = match session.run(&samples, &options) {
                Ok(transcript) => transcript,
                Err(error) => {
                    // A decode that stopped early keeps the words it found.
                    let partial = match error.kind() {
                        ErrorKind::OutputTruncated | ErrorKind::OutputRepetition => {
                            error.partial().cloned()
                        }
                        _ => None,
                    };
                    match partial {
                        Some(partial) => partial,
                        None => return Err(error.into()),
                    }
                }
            };
            Ok(timed_words(
                transcript
                    .words
                    .into_iter()
                    .map(|word| (word.text, word.t0_ms, word.t1_ms)),
            ))
        })
    }
}

/// The words of a piece as the live transcript takes them: trimmed, empty
/// ones dropped, times in seconds from the start of the piece.
fn timed_words(words: impl IntoIterator<Item = (String, i64, i64)>) -> Vec<Word> {
    words
        .into_iter()
        .filter_map(|(text, start_ms, end_ms)| {
            let text = text.trim();
            (!text.is_empty()).then(|| Word {
                w: text.to_string(),
                s: start_ms as f64 / 1000.0,
                e: end_ms.max(start_ms) as f64 / 1000.0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_words_are_trimmed_timed_in_seconds_and_never_end_before_they_start() {
        let heard = timed_words([
            (" Hello".to_string(), 500, 900),
            ("  ".to_string(), 900, 950),
            ("world.".to_string(), 1_000, 800),
        ]);
        assert_eq!(
            heard,
            [
                Word {
                    w: "Hello".into(),
                    s: 0.5,
                    e: 0.9
                },
                Word {
                    w: "world.".into(),
                    s: 1.0,
                    e: 1.0
                },
            ]
        );
    }

    /// Needs the models and spoken clips, so CI runs it on its own (`local
    /// engine` on Intel, `windows beta` on Windows):
    /// `MURMUR_SMOKE_MODELS=<models dir> MURMUR_SMOKE_CLIPS=<dir> cargo test
    /// --target x86_64-apple-darwin --lib engine::local::cpu -- --ignored`.
    /// Each `<name>.wav` (16 kHz mono) in the folder holds speech whose words
    /// are listed, one per line, in `<name>.txt`.
    #[tokio::test]
    #[ignore = "needs the GGUF model and recorded speech"]
    async fn the_cpu_model_hears_the_words_of_each_clip() {
        let models = std::env::var("MURMUR_SMOKE_MODELS").expect("MURMUR_SMOKE_MODELS");
        let clips = PathBuf::from(std::env::var("MURMUR_SMOKE_CLIPS").expect("MURMUR_SMOKE_CLIPS"));
        let started = std::time::Instant::now();
        let mut loaded = CpuRuntime {
            models: models.into(),
        }
        .load()
        .await
        .unwrap();
        println!("loaded in {:?}", started.elapsed());
        let mut checked = 0;
        for entry in std::fs::read_dir(&clips).unwrap() {
            let wav = entry.unwrap().path();
            if wav.extension().is_none_or(|extension| extension != "wav") {
                continue;
            }
            let samples = crate::audio_toolkit::read_wav_samples(&wav).unwrap();
            let started = std::time::Instant::now();
            let text = loaded
                .transcribe(&samples, &[])
                .await
                .unwrap()
                .to_lowercase();
            // transcribe.cpp drops the `<unk>` Parakeet Ultra writes
            // (`keep_special_tags` is off), as `unknown` does on Apple silicon.
            assert!(!text.contains("<unk>"), "{text}");
            println!(
                "{}: {:.1} s of audio in {:?}: {text}",
                wav.display(),
                samples.len() as f64 / 16_000.0,
                started.elapsed()
            );
            let words = std::fs::read_to_string(wav.with_extension("txt")).unwrap();
            for word in words.lines().filter(|word| !word.is_empty()) {
                assert!(text.contains(&word.to_lowercase()), "{word} missing");
            }
            checked += 1;
        }
        assert!(checked > 0, "no clips in {}", clips.display());
    }

    /// The loaded model maps its file: a download renaming a new file over
    /// it, then the models being deleted, leave it hearing the same words.
    /// Same variables as above. macOS only: Windows may refuse both while
    /// the file is mapped, so a delete releases the model first
    /// (`deleting_the_models_releases_the_loaded_cpu_model`).
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "needs the GGUF model and recorded speech"]
    async fn the_loaded_model_survives_its_file_replaced_then_deleted() {
        let gguf = crate::local_models::manifest::Asr::Current.gguf_path();
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join(&gguf);
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::copy(
            PathBuf::from(std::env::var("MURMUR_SMOKE_MODELS").expect("MURMUR_SMOKE_MODELS"))
                .join(&gguf),
            &model,
        )
        .unwrap();
        let clips = PathBuf::from(std::env::var("MURMUR_SMOKE_CLIPS").expect("MURMUR_SMOKE_CLIPS"));
        let wav = std::fs::read_dir(&clips)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "wav"))
            .expect("no clip");
        let samples = crate::audio_toolkit::read_wav_samples(&wav).unwrap();
        let mut loaded = CpuRuntime {
            models: dir.path().to_path_buf(),
        }
        .load()
        .await
        .unwrap();
        let heard = loaded.transcribe(&samples, &[]).await.unwrap();
        assert!(!heard.is_empty());

        let part = crate::local_models::download::part_path(&model);
        std::fs::write(&part, vec![0u8; 1 << 20]).unwrap();
        std::fs::rename(&part, &model).unwrap();
        assert_eq!(loaded.transcribe(&samples, &[]).await.unwrap(), heard);

        std::fs::remove_file(&model).unwrap();
        assert_eq!(loaded.transcribe(&samples, &[]).await.unwrap(), heard);
        println!("still heard after replace and delete: {heard}");
    }

    /// `deleting_the_models_releases_the_mapped_model` with the real model,
    /// mapped by transcribe.cpp: the delete removes the file and the next
    /// dictation loads the model again. NTFS deletes a mapped file, so on
    /// Windows this does not prove the release comes first; the other test
    /// does, by holding the file. `MURMUR_SMOKE_MODELS` as above.
    #[tokio::test]
    #[ignore = "needs the GGUF model"]
    async fn deleting_the_models_releases_the_loaded_cpu_model() {
        let gguf = crate::local_models::manifest::Asr::Current.gguf_path();
        let source =
            PathBuf::from(std::env::var("MURMUR_SMOKE_MODELS").expect("MURMUR_SMOKE_MODELS"))
                .join(&gguf);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models");
        let (models, _) = crate::local_models::tests::ready_models(&path);
        let model = path.join(&gguf);
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::copy(&source, &model).unwrap();
        let engine = crate::engine::local::LocalEngine::new(
            Box::new(CpuRuntime {
                models: path.clone(),
            }),
            crate::engine::local::MAX_PIECE,
            Box::new(|| false),
        );
        engine.session.warm().await.unwrap();

        crate::commands::local_models::delete(Arc::clone(&models), Some(&engine))
            .await
            .unwrap();
        assert!(!model.exists());
        assert!(!engine.session.is_loaded().await);

        assert!(engine.session.warm().await.is_err());
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::copy(&source, &model).unwrap();
        engine.session.warm().await.unwrap();
        assert!(engine.session.is_loaded().await);
    }
}
