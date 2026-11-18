use crate::audio_toolkit::gain::{normalize_quiet_speech, GainProfile};
use crate::audio_toolkit::text_passes::{self, PassLanguage};
use crate::audio_toolkit::{apply_custom_words_counted, normalize_transcription_output};
use crate::codex_stream::{self, LiveDictation, Stop, Wanted};
use crate::codex_transcribe;
use crate::engine::local::boost::boost_terms;
use crate::engine::local::LocalEngine;
use crate::engine::{self, Conditions, Engine, EngineUsed, LocalReadiness, Route};
use crate::local_models::{LocalModelState, LocalModels};
use crate::settings::{get_settings, AppSettings};
use crate::{OperationId, ProcessingOperation};
use anyhow::Result;
use log::{debug, error, info};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};

/// Final dictation text plus how many dictionary replacements produced it.
#[derive(Debug, Default)]
pub struct Transcript {
    pub text: String,
    pub dictionary_fixes: u32,
    pub source: EngineUsed,
    /// The language the text passes ran in, if they ran.
    pub pass_language: Option<PassLanguage>,
}

/// The stream of the recording being transcribed.
struct Streamed {
    live: LiveDictation,
    /// The engine gets the audio at a raised level, which the stream did not
    /// hear.
    audio_raised: bool,
}

/// ChatGPT's text for a dictation: its stream's when that can be used, else
/// the upload's. The stream is asked once and closed either way.
async fn stream_or_upload<Fut>(
    streamed: Option<(LiveDictation, Stop<'_>)>,
    operation: &ProcessingOperation,
    upload: impl FnOnce() -> Fut,
) -> Result<String>
where
    Fut: Future<Output = Result<String>>,
{
    if let Some((live, stop)) = streamed {
        if let Some(text) = live.text(&stop, operation).await {
            return Ok(text);
        }
    }
    upload().await
}

/// Transcribes `audio` with `engine`. `language` is the dictation setting.
/// Only the local engine listens for `terms`: the Codex endpoint takes none.
/// "auto" names no language to either engine. Only Codex asks `streamed`.
async fn run(
    engine: Engine,
    local: Option<&LocalEngine>,
    audio: &[f32],
    language: &str,
    terms: &[String],
    operation: &ProcessingOperation,
    streamed: Option<Streamed>,
) -> Result<String> {
    let language = (language != "auto").then_some(language);
    match engine {
        Engine::Cloud => {
            let streamed = streamed.map(|Streamed { live, audio_raised }| {
                let stop = Stop {
                    audio_raised,
                    language: language.and_then(codex_transcribe::request_language),
                };
                (live, stop)
            });
            stream_or_upload(streamed, operation, || async {
                debug!(
                    "Sending {} samples to Codex transcription (language={:?})",
                    audio.len(),
                    language
                );
                codex_transcribe::transcribe(audio, language, operation).await
            })
            .await
            .inspect_err(|err| error!("Codex transcription failed: {err}"))
        }
        Engine::Local => {
            let local = local.ok_or_else(|| anyhow::anyhow!("No local engine in this build"))?;
            debug!(
                "Transcribing {} samples on this Mac with {} boost term(s)",
                audio.len(),
                terms.len()
            );
            local.transcribe(audio, terms, language, operation).await
        }
    }
}

/// Settles a request that needs no engine: empty audio gives an empty
/// transcript, a cancelled operation an error. `None` sends it on.
fn settle_without_engine(
    audio: &[f32],
    operation: &ProcessingOperation,
) -> Option<Result<Transcript>> {
    if audio.is_empty() {
        return Some(Ok(Transcript::default()));
    }
    if operation.is_cancelled() {
        return Some(Err(anyhow::anyhow!("Transcription cancelled")));
    }
    None
}

/// The requests in flight, so that shutdown can cancel them.
struct ActiveRequests {
    active_operations: Arc<Mutex<HashMap<u64, ProcessingOperation>>>,
    next_active_operation: AtomicU64,
    shutdown_requested: AtomicBool,
}

impl ActiveRequests {
    fn new() -> Self {
        Self {
            active_operations: Arc::new(Mutex::new(HashMap::new())),
            next_active_operation: AtomicU64::new(1),
            shutdown_requested: AtomicBool::new(false),
        }
    }

    /// Tracks `operation` until the guard drops. After shutdown the
    /// operation is cancelled and refused.
    fn register(&self, operation: &ProcessingOperation) -> Result<ActiveOperationGuard> {
        let id = self.next_active_operation.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut operations) = self.active_operations.lock() {
            operations.insert(id, operation.clone());
        }
        let guard = ActiveOperationGuard {
            id,
            active_operations: Arc::clone(&self.active_operations),
        };
        if self.shutdown_requested.load(Ordering::Acquire) {
            operation.cancel();
            return Err(anyhow::anyhow!("Transcription manager is shutting down"));
        }
        Ok(guard)
    }

    /// Cancels every active request and refuses new ones, without waiting.
    fn shutdown(&self) {
        self.shutdown_requested.store(true, Ordering::Release);
        let active_operations = self
            .active_operations
            .lock()
            .map(|operations| operations.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for operation in active_operations {
            operation.cancel();
        }
    }
}

/// Sends audio to the local engine or Codex.
pub struct TranscriptionManager {
    app_handle: AppHandle,
    requests: ActiveRequests,
}

struct ActiveOperationGuard {
    id: u64,
    active_operations: Arc<Mutex<HashMap<u64, ProcessingOperation>>>,
}

impl Drop for ActiveOperationGuard {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.active_operations.lock() {
            operations.remove(&self.id);
        }
    }
}

/// `settings` with the dictation language replaced by `language`, when one is
/// given. Every later read of the language then sees the one-off choice.
pub(crate) fn with_language(mut settings: AppSettings, language: Option<&str>) -> AppSettings {
    if let Some(language) = language {
        settings.selected_language = language.to_string();
    }
    settings
}

impl TranscriptionManager {
    /// Creates a transcription manager bound to the current Tauri app.
    pub fn new(app_handle: &AppHandle) -> Self {
        Self {
            app_handle: app_handle.clone(),
            requests: ActiveRequests::new(),
        }
    }

    /// Transcribes shared samples and corrects them with the dictionary.
    /// The local engine listens for the custom words.
    pub(crate) async fn transcribe_samples(
        &self,
        audio: Arc<Vec<f32>>,
        operation: ProcessingOperation,
    ) -> Result<Transcript> {
        self.transcribe_samples_in(audio, operation, None, None)
            .await
    }

    /// Like `transcribe_samples`, in `language` for this call only when one
    /// is given ("auto" included): the language setting is neither read nor
    /// changed for it, so the engine route, the text passes and the language
    /// the text is judged in all follow `language`. `live` is the stream of
    /// the recording that gave `audio`: ChatGPT's text comes from it when it
    /// can, and it is closed whatever happens.
    pub(crate) async fn transcribe_samples_in(
        &self,
        audio: Arc<Vec<f32>>,
        operation: ProcessingOperation,
        language: Option<&str>,
        live: Option<LiveDictation>,
    ) -> Result<Transcript> {
        let _active_operation = self.requests.register(&operation)?;
        #[cfg(debug_assertions)]
        if std::env::var("MURMUR_FORCE_TRANSCRIPTION_FAILURE").is_ok() {
            return Err(anyhow::anyhow!(
                "Simulated transcription failure (MURMUR_FORCE_TRANSCRIPTION_FAILURE)"
            ));
        }

        if let Some(settled) = settle_without_engine(&audio, &operation) {
            return settled;
        }

        let settings = with_language(get_settings(&self.app_handle), language);
        let conditions = self.conditions(&settings);
        // The saved WAV keeps the original level; only the engine's input is
        // raised.
        let louder = normalize_quiet_speech(&audio, GainProfile::new(settings.whisper_mode));
        let mut streamed = live.map(|live| Streamed {
            live,
            audio_raised: louder.is_some(),
        });
        let audio = match louder {
            Some(louder) => {
                debug!("Raised the level of quiet speech before transcription");
                Arc::new(louder)
            }
            None => audio,
        };

        let local = self.app_handle.try_state::<Arc<LocalEngine>>();
        let local = local.as_deref().map(Arc::as_ref);
        let terms = boost_terms(&settings.custom_words);
        let (text, source) = engine::transcribe(
            settings.transcription_engine,
            conditions,
            || operation.is_cancelled(),
            |engine| {
                let streamed = streamed.take().filter(|_| engine == Engine::Cloud);
                run(
                    engine,
                    local,
                    &audio,
                    conditions.language,
                    &terms,
                    &operation,
                    streamed,
                )
            },
        )
        .await?;

        if operation.is_cancelled() {
            return Err(anyhow::anyhow!("Transcription cancelled"));
        }

        let (processed, dictionary_fixes) = apply_custom_words_counted(
            &normalize_transcription_output(&text),
            &settings.custom_words,
            settings.word_correction_threshold,
        );
        let pass_language = text_passes::detect(&settings.selected_language, &processed);
        let processed = match pass_language {
            Some(language) => text_passes::before_cleanup(&processed, language),
            None => processed,
        };

        info!(
            "The {} engine produced {} characters",
            source.engine.as_str(),
            processed.len()
        );
        Ok(Transcript {
            text: processed,
            dictionary_fixes,
            source,
            pass_language,
        })
    }

    fn conditions<'a>(&self, settings: &'a AppSettings) -> Conditions<'a> {
        let engine = self.app_handle.try_state::<Arc<LocalEngine>>().is_some();
        let state = self
            .app_handle
            .try_state::<Arc<LocalModels>>()
            .map(|models| models.state());
        let model = match state {
            Some(LocalModelState::Ready) if engine => LocalReadiness::Ready,
            Some(LocalModelState::Optimizing) if engine => LocalReadiness::Optimizing,
            _ => LocalReadiness::NotReady,
        };
        Conditions {
            model,
            signed_in: crate::codex_auth::auth_status().signed_in,
            language: &settings.selected_language,
        }
    }

    /// At the start of a recording: loads the local model when the dictation
    /// will go to it, so the speaking time hides the load.
    pub fn warm_if_local(&self) {
        let settings = get_settings(&self.app_handle);
        let route = engine::route(settings.transcription_engine, self.conditions(&settings));
        if matches!(
            route,
            Route::Run(EngineUsed {
                engine: Engine::Local,
                ..
            })
        ) {
            if let Some(local) = self.app_handle.try_state::<Arc<LocalEngine>>() {
                local.warm();
            }
        }
    }

    /// At the start of a recording: opens the parked stream to ChatGPT when
    /// this dictation is to go there, in the general dictation language. From
    /// then on the audio leaves this Mac while the user speaks. Otherwise the
    /// audio that waited for the stream is dropped.
    pub fn open_cloud_stream(&self) {
        let settings = get_settings(&self.app_handle);
        let conditions = self.conditions(&settings);
        let route = engine::route(settings.transcription_engine, conditions);
        codex_stream::open_parked(&Wanted {
            enabled: settings.cloud_stream_enabled,
            cloud: matches!(
                route,
                Route::Run(EngineUsed {
                    engine: Engine::Cloud,
                    ..
                })
            ),
            signed_in: conditions.signed_in,
            language: &settings.selected_language,
        });
    }

    /// At the start of a recording: opens the connection to ChatGPT when this
    /// dictation will use it (the cloud engine, or Codex cleanup), so the
    /// handshake overlaps the speaking time. Sends no audio and no text.
    pub fn warm_cloud_connection(&self) {
        let settings = get_settings(&self.app_handle);
        let route = engine::route(settings.transcription_engine, self.conditions(&settings));
        let transcribes = matches!(
            route,
            Route::Run(EngineUsed {
                engine: Engine::Cloud,
                ..
            })
        );
        if transcribes || (settings.ai_cleanup_enabled && crate::cleanup::available()) {
            crate::http::warm_up(crate::codex_transcribe::TRANSCRIBE_URL);
        }
    }

    /// Keeps CLI calls synchronous and independent of desktop cancellation.
    pub fn transcribe_sync(&self, audio: Vec<f32>) -> Result<Transcript> {
        let operation = ProcessingOperation::new(OperationId(0));
        tauri::async_runtime::block_on(self.transcribe_samples(Arc::new(audio), operation))
    }

    /// Cancels every active request and refuses new ones, without waiting.
    pub fn shutdown(&self) {
        self.requests.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::{settle_without_engine, stream_or_upload, with_language, ActiveRequests};
    use crate::codex_stream::fake_server::{closed, serve, started, transcript_failed, Script};
    use crate::codex_stream::{self, LiveDictation, Stop};
    use crate::codex_transcribe::tests::{serve_bodies, signed_in, TRANSCRIBED};
    use crate::codex_transcribe::{self, to_pcm16, uploaded};
    use crate::settings::get_default_settings;
    use crate::{OperationId, ProcessingOperation};
    use std::time::Duration;

    fn operation() -> ProcessingOperation {
        ProcessingOperation::new(OperationId(1))
    }

    #[test]
    fn empty_audio_gives_an_empty_transcript_before_any_engine() {
        let settled = settle_without_engine(&[], &operation());
        let transcript = settled.expect("settled").expect("empty transcript");
        assert_eq!(transcript.text, "");
        assert_eq!(transcript.dictionary_fixes, 0);
    }

    #[test]
    fn cancelled_audio_fails_before_any_engine() {
        let cancelled = operation();
        cancelled.cancel();
        let settled = settle_without_engine(&[0.1, 0.2], &cancelled).expect("settled");
        assert!(settled.is_err());
    }

    #[test]
    fn a_one_off_language_replaces_the_setting_only_in_the_copy() {
        let mut stored = get_default_settings();
        stored.selected_language = "fr".to_string();

        assert_eq!(
            with_language(stored.clone(), Some("de")).selected_language,
            "de"
        );
        assert_eq!(
            with_language(stored.clone(), Some("auto")).selected_language,
            "auto"
        );
        assert_eq!(with_language(stored.clone(), None).selected_language, "fr");
        assert_eq!(stored.selected_language, "fr");
    }

    #[test]
    fn live_audio_goes_on_to_an_engine() {
        assert!(settle_without_engine(&[0.1, 0.2], &operation()).is_none());
    }

    #[test]
    fn shutdown_cancels_a_request_in_flight() {
        let requests = ActiveRequests::new();
        let in_flight = operation();
        let _guard = requests.register(&in_flight).expect("accepted");
        assert!(!in_flight.is_cancelled());

        requests.shutdown();
        assert!(in_flight.is_cancelled());
    }

    #[test]
    fn after_shutdown_the_next_request_fails_and_is_cancelled() {
        let requests = ActiveRequests::new();
        let before = operation();
        assert!(requests.register(&before).is_ok());

        requests.shutdown();
        let next = operation();
        assert!(requests.register(&next).is_err());
        assert!(next.is_cancelled());
    }

    #[test]
    fn shutdown_leaves_a_finished_request_alone() {
        let requests = ActiveRequests::new();
        let finished = operation();
        drop(requests.register(&finished).expect("accepted"));

        requests.shutdown();
        assert!(!finished.is_cancelled());
    }

    const AS_STARTED: Stop<'static> = Stop {
        audio_raised: false,
        language: None,
    };

    /// Half a second of a recording.
    fn recording() -> Vec<f32> {
        (0..8_000).map(|i| (i % 800) as f32 / 1_600.0).collect()
    }

    /// The dictation of `recording` as a server playing `script` streamed
    /// it, once its recording stopped.
    fn streamed(script: Script) -> LiveDictation {
        let (mut tap, live) = serve(script).dictation(None);
        for frame in recording().chunks(480) {
            tap(frame);
        }
        live
    }

    /// ChatGPT's text for `recording`, and the uploads it took.
    fn transcribed(
        streamed: Option<(LiveDictation, Stop<'_>)>,
        operation: &ProcessingOperation,
    ) -> (anyhow::Result<String>, Vec<Vec<u8>>) {
        let directory = tempfile::tempdir().unwrap();
        let auth_path = signed_in(directory.path());
        let (upload_url, uploads) = serve_bodies(&[TRANSCRIBED]);
        let samples = recording();
        let text = tauri::async_runtime::block_on(stream_or_upload(streamed, operation, || {
            codex_transcribe::transcribe_at(
                &upload_url,
                Some(&auth_path),
                &samples,
                None,
                1,
                operation,
            )
        }));
        let mut requests = Vec::new();
        while let Ok(body) = uploads.recv_timeout(Duration::from_millis(300)) {
            requests.push(body);
        }
        (text, requests)
    }

    #[test]
    fn a_dictation_whose_stream_gave_the_text_is_not_uploaded() {
        let live = streamed(Script::transcribing("Bonjour à tous"));

        let (text, uploads) = transcribed(Some((live, AS_STARTED)), &operation());

        assert_eq!(text.unwrap(), "Bonjour à tous.");
        assert!(uploads.is_empty(), "the clip was uploaded as well");
    }

    #[test]
    fn a_dictation_whose_stream_is_not_used_is_uploaded_once_with_its_audio() {
        let well = || Script::transcribing("Bonjour à tous");
        let raised = Stop {
            audio_raised: true,
            language: None,
        };
        let in_english = Stop {
            audio_raised: false,
            language: Some("en"),
        };
        let rows = [
            ("no stream", None),
            (
                "a stream nothing opened",
                Some((codex_stream::pair().1, AS_STARTED)),
            ),
            (
                "audio raised for the engine",
                Some((streamed(well()), raised)),
            ),
            ("another language", Some((streamed(well()), in_english))),
            (
                "a refused handshake",
                Some((
                    streamed(Script {
                        refuse: Some(403),
                        ..Script::default()
                    }),
                    AS_STARTED,
                )),
            ),
            (
                "a transcript that failed",
                Some((
                    streamed(Script {
                        on_close: vec![transcript_failed()],
                        ..well()
                    }),
                    AS_STARTED,
                )),
            ),
            (
                "a session without text",
                Some((
                    streamed(Script {
                        on_start: vec![started()],
                        on_close: vec![closed()],
                        ..Script::default()
                    }),
                    AS_STARTED,
                )),
            ),
        ];
        let audio: Vec<i16> = recording().iter().map(|sample| to_pcm16(*sample)).collect();
        for (case, streamed) in rows {
            let (text, uploads) = transcribed(streamed, &operation());

            assert_eq!(text.unwrap(), "Hello", "{case}");
            assert_eq!(uploads.len(), 1, "{case}");
            let heard = uploaded::pcm_of_request(&uploads[0]).expect("an audio file");
            let differs = heard.iter().zip(&audio).position(|(a, b)| a != b);
            assert!(
                heard == audio,
                "{case}: {} samples against {}, first difference at {differs:?}",
                heard.len(),
                audio.len()
            );
        }
    }

    #[test]
    fn a_cancelled_dictation_is_neither_asked_of_its_stream_nor_uploaded() {
        let live = streamed(Script::transcribing("Bonjour à tous"));
        let cancelled = operation();
        cancelled.cancel();

        let (text, uploads) = transcribed(Some((live, AS_STARTED)), &cancelled);

        assert!(text.unwrap_err().to_string().contains("cancelled"));
        assert!(uploads.is_empty());
    }
}
