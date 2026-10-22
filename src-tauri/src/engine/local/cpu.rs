//! Intel Macs: the GGUF model (8-bit) through transcribe.cpp, on the CPU in
//! the app. Forced to the CPU: Intel Macs have no GPU it can use.
//! Boost terms are not passed: transcribe-cpp 0.4.0's Parakeet does not set
//! `TRANSCRIBE_FEATURE_VOCABULARY`, so its `RunOptions.vocabulary` is dropped.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use transcribe_cpp::{Backend, CancelToken, Model, ModelOptions, RunOptions, Session};

use super::boost::BoostTerms;
use super::session::{BoxFuture, Loaded, Runtime};

pub struct CpuRuntime {
    pub model: PathBuf,
}

impl Runtime for CpuRuntime {
    fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
        let path = self.model.clone();
        Box::pin(async move {
            let model = tokio::task::spawn_blocking(move || -> anyhow::Result<CpuModel> {
                let options = ModelOptions {
                    backend: Backend::Cpu,
                    device: None,
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

struct CpuModel {
    /// Shared with the thread of a run, which may outlive an abandoned one.
    session: Arc<Mutex<Session>>,
    cancel: CancelToken,
}

/// Stops the run on its thread when the request is abandoned.
struct AbortOnDrop(Option<CancelToken>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.cancel();
        }
    }
}

impl Loaded for CpuModel {
    fn transcribe<'a>(
        &'a mut self,
        samples: &'a [f32],
        _terms: &'a BoostTerms,
    ) -> BoxFuture<'a, anyhow::Result<String>> {
        let session = Arc::clone(&self.session);
        let cancel = self.cancel.clone();
        let samples = samples.to_vec();
        Box::pin(async move {
            let mut abort = AbortOnDrop(Some(cancel.clone()));
            let text = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
                let mut session = session
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                cancel.reset();
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
                .transcribe(&samples, &BoostTerms::default())
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
}
