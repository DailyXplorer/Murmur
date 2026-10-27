//! Intel Macs: the GGUF model (8-bit) through transcribe.cpp, on the CPU in
//! the app. Forced to the CPU: Intel Macs have no GPU it can use.
//! Boost terms are not passed: transcribe-cpp 0.4.1's Parakeet does not set
//! `TRANSCRIBE_FEATURE_VOCABULARY`, so its `RunOptions.vocabulary` is dropped.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use transcribe_cpp::{Backend, CancelToken, Model, ModelOptions, RunOptions, Session};

use super::session::{BoxFuture, Loaded, Runtime};

pub struct CpuRuntime {
    pub model: PathBuf,
}

impl Runtime for CpuRuntime {
    fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
        let path = self.model.clone();
        Box::pin(async move {
            let model = tokio::task::spawn_blocking(move || -> anyhow::Result<CpuModel> {
                #[cfg(target_os = "windows")]
                load_cpu_modules()?;
                let options = ModelOptions {
                    backend: Backend::Cpu,
                    device: None,
                    // Mapped from the file: the model is never rewritten in
                    // place, only replaced by a rename.
                    no_mmap: false,
                };
                let mut session = Model::load_with(&path, &options)?.session()?;
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

/// Windows ships the CPU code as one module per instruction set, beside the
/// app (see `build.rs`); transcribe.cpp loads the fastest this processor
/// runs. Once per process, before the first model load: it cannot be retried.
#[cfg(target_os = "windows")]
fn load_cpu_modules() -> anyhow::Result<()> {
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

impl Loaded for CpuModel {
    fn transcribe<'a>(
        &'a mut self,
        samples: &'a [f32],
        _terms: &'a [String],
    ) -> BoxFuture<'a, anyhow::Result<String>> {
        let session = Arc::clone(&self.session);
        let cancel = self.cancel.clone();
        let samples = samples.to_vec();
        Box::pin(async move {
            let abandoned = Arc::new(AtomicBool::new(false));
            let mut abort = AbortOnDrop(Some((cancel.clone(), Arc::clone(&abandoned))));
            let text = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
                let mut session = session
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                cancel.reset();
                // Read after the reset: a later abandon cancels the run.
                if abandoned.load(Ordering::SeqCst) {
                    anyhow::bail!("Local transcription abandoned");
                }
                Ok(session.run(&samples, &RunOptions::default())?.text)
            })
            .await??;
            abort.0 = None;
            Ok(text)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the model and spoken clips, so CI runs it on its own:
    /// `MURMUR_SMOKE_GGUF=<model> MURMUR_SMOKE_CLIPS=<dir> cargo test
    /// --target x86_64-apple-darwin --lib engine::local::cpu -- --ignored`.
    /// Each `<name>.wav` (16 kHz mono) in the folder holds speech whose words
    /// are listed, one per line, in `<name>.txt`.
    #[tokio::test]
    #[ignore = "needs the GGUF model and recorded speech"]
    async fn the_cpu_model_hears_the_words_of_each_clip() {
        let model = std::env::var("MURMUR_SMOKE_GGUF").expect("MURMUR_SMOKE_GGUF");
        let clips = PathBuf::from(std::env::var("MURMUR_SMOKE_CLIPS").expect("MURMUR_SMOKE_CLIPS"));
        let started = std::time::Instant::now();
        let mut loaded = CpuRuntime {
            model: model.into(),
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
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.gguf");
        std::fs::copy(
            std::env::var("MURMUR_SMOKE_GGUF").expect("MURMUR_SMOKE_GGUF"),
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
            model: model.clone(),
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
    /// dictation loads the model again. `MURMUR_SMOKE_GGUF` as above.
    #[tokio::test]
    #[ignore = "needs the GGUF model"]
    async fn deleting_the_models_releases_the_loaded_cpu_model() {
        let gguf = std::env::var("MURMUR_SMOKE_GGUF").expect("MURMUR_SMOKE_GGUF");
        let dir = tempfile::tempdir().unwrap();
        let (models, model) = crate::local_models::tests::ready_models(&dir.path().join("models"));
        std::fs::copy(&gguf, &model).unwrap();
        let engine = crate::engine::local::LocalEngine::new(
            Box::new(CpuRuntime {
                model: model.clone(),
            }),
            crate::engine::local::MAX_PIECE,
            Box::new(|| false),
        );
        engine.session.warm().await.unwrap();
        // What this Windows does with the file transcribe.cpp maps.
        #[cfg(target_os = "windows")]
        println!(
            "deleting the mapped model: {:?}",
            std::fs::remove_file(&model)
        );

        crate::commands::local_models::delete(Arc::clone(&models), Some(&engine))
            .await
            .unwrap();
        assert!(!model.exists());
        assert!(!engine.session.is_loaded().await);

        assert!(engine.session.warm().await.is_err());
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::copy(&gguf, &model).unwrap();
        engine.session.warm().await.unwrap();
        assert!(engine.session.is_loaded().await);
    }
}
