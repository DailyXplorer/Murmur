//! Apple silicon: the meetings helper's `serve` mode keeps FluidAudio's
//! Parakeet loaded and answers one request per line. Each piece crosses as a
//! WAV in the app's cache, deleted once answered. The helper stays a separate
//! process so a Core ML crash cannot take the app down, and killing it is how
//! its memory is released.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;

use super::session::{BoxFuture, Heard, Loaded, Runtime};
use super::SAMPLES_PER_SECOND;
use crate::local_models::LocalModelState;
use crate::meetings::protocol::{parse_reply, Reply, Request, Word};

/// A load from compiled models takes well under a second, one that compiles
/// them about 25 s. The prewarm run counts in it.
const LOAD_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Unique across the dictation and meeting sessions, which share the folder.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct FluidRuntime {
    program: PathBuf,
    models: PathBuf,
    scratch: PathBuf,
    state: watch::Receiver<LocalModelState>,
}

impl FluidRuntime {
    pub fn new(
        program: PathBuf,
        models: PathBuf,
        scratch: PathBuf,
        state: watch::Receiver<LocalModelState>,
    ) -> Self {
        Self {
            program,
            models,
            scratch,
            state,
        }
    }
}

impl Runtime for FluidRuntime {
    fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
        let (program, models, scratch, mut state) = (
            self.program.clone(),
            self.models.clone(),
            self.scratch.clone(),
            self.state.clone(),
        );
        Box::pin(async move {
            // Loading during the compile after a macOS update would compile
            // the models a second time, beside it.
            let _ = state
                .wait_for(|state| *state != LocalModelState::Optimizing)
                .await;
            std::fs::create_dir_all(&scratch)?;
            let spawned = Instant::now();
            let mut serve = Serve::spawn(&program, &models, scratch)?;
            let loaded = async {
                serve.ready().await?;
                log::info!("Local model ready in {} ms", spawned.elapsed().as_millis());
                // The first run after a load is slower: this one takes that
                // cost off the dictation, and finds a broken model before it.
                let prewarm = Instant::now();
                serve
                    .transcribe(&vec![0.0; SAMPLES_PER_SECOND], &[], None)
                    .await?;
                log::info!("Local model prewarm {} ms", prewarm.elapsed().as_millis());
                anyhow::Ok(())
            };
            match tokio::time::timeout(LOAD_TIMEOUT, loaded).await {
                Ok(result) => result?,
                Err(_) => anyhow::bail!("The helper did not load the models in {LOAD_TIMEOUT:?}"),
            }
            Ok(Box::new(serve) as Box<dyn Loaded>)
        })
    }
}

struct Serve {
    /// Killed when dropped; closing stdin alone also makes it quit.
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    scratch: PathBuf,
    models: PathBuf,
    /// The custom-word model was in place when the helper started.
    boosting: bool,
}

/// Deletes a piece's WAV once it is answered, or abandoned.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Serve {
    fn spawn(program: &Path, models: &Path, scratch: PathBuf) -> std::io::Result<Self> {
        let boosting = crate::local_models::optional_ready(models);
        let mut child = Command::new(program)
            .arg("serve")
            .arg("--models-dir")
            .arg(models)
            .arg("--asr")
            .arg(crate::local_models::asr_in_use(models).helper_name())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("Local transcription helper: {line}");
                }
            });
        }
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(std::io::Error::other("the helper has no stdin or stdout"));
        };
        Ok(Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            scratch,
            models: models.to_path_buf(),
            boosting,
        })
    }

    /// The next reply. Lines that are not replies are skipped; none is ever
    /// logged, as they may hold dictated text.
    async fn reply(&mut self) -> anyhow::Result<Reply> {
        loop {
            let Some(line) = self.stdout.next_line().await? else {
                anyhow::bail!("The local transcription helper stopped");
            };
            if line.trim().is_empty() {
                continue;
            }
            match parse_reply(&line) {
                Ok(reply) => return Ok(reply),
                Err(error) => log::warn!("The helper printed an unreadable line: {error}"),
            }
        }
    }

    async fn ready(&mut self) -> anyhow::Result<()> {
        loop {
            match self.reply().await? {
                Reply::Ready => return Ok(()),
                Reply::Error { code, message, .. } => {
                    anyhow::bail!("The helper cannot load the models ({code:?}): {message}")
                }
                Reply::Text { .. } | Reply::Unknown => {}
            }
        }
    }
}

impl Loaded for Serve {
    /// The helper loads the custom-word model only as it starts.
    fn outdated(&self) -> bool {
        !self.boosting && crate::local_models::optional_ready(&self.models)
    }

    /// The helper boosts `terms` with FluidAudio's CTC keyword spotting when
    /// the optional CTC model is downloaded, and otherwise ignores them.
    /// `language` keeps Parakeet's text in that language.
    fn transcribe<'a>(
        &'a mut self,
        samples: &'a [f32],
        terms: &'a [String],
        language: Option<&'a str>,
    ) -> BoxFuture<'a, anyhow::Result<Heard>> {
        Box::pin(async move {
            let (text, words) = self.request(samples, terms, language, false).await?;
            Ok(Heard {
                text,
                words: words.into_iter().map(|word| word.s..word.e).collect(),
            })
        })
    }

    fn transcribe_words<'a>(
        &'a mut self,
        samples: &'a [f32],
    ) -> BoxFuture<'a, anyhow::Result<Vec<Word>>> {
        Box::pin(async move { Ok(self.request(samples, &[], None, true).await?.1) })
    }
}

impl Serve {
    /// One request. A live meeting's runs at background priority, a
    /// dictation's at the highest; both ask for the word times.
    async fn request(
        &mut self,
        samples: &[f32],
        terms: &[String],
        language: Option<&str>,
        live: bool,
    ) -> anyhow::Result<(String, Vec<Word>)> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let wav = Scratch(self.scratch.join(format!("piece-{id}.wav")));
        crate::audio_toolkit::save_wav_file(&wav.0, samples)?;
        let mut line = serde_json::to_string(&Request::Transcribe {
            id,
            wav: &wav.0,
            terms,
            background: live.then_some(true),
            language,
            words: Some(true),
        })?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        // Answers to an abandoned request may come first.
        loop {
            match self.reply().await? {
                Reply::Text {
                    id: answered,
                    text,
                    words,
                } if answered == id => return Ok((text, words.unwrap_or_default())),
                Reply::Error {
                    id: Some(answered),
                    code,
                    message,
                } if answered == id => {
                    anyhow::bail!("Local transcription failed ({code:?}): {message}")
                }
                Reply::Error {
                    id: None, message, ..
                } => anyhow::bail!("The local transcription helper failed: {message}"),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::session::Session;
    use super::*;
    use crate::engine::{route, Conditions, Engine, EngineUsed, LocalReadiness, Route};
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    /// A stand-in helper: a shell script with the given body. `$wav` and
    /// `$id` hold the fields of each request it reads.
    fn runtime(dir: &Path, body: &str) -> FluidRuntime {
        runtime_while(dir, body, watch::channel(LocalModelState::Ready).1)
    }

    fn runtime_while(
        dir: &Path,
        body: &str,
        state: watch::Receiver<LocalModelState>,
    ) -> FluidRuntime {
        let program = dir.join("helper.sh");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        FluidRuntime::new(program, dir.join("models"), dir.join("scratch"), state)
    }

    /// Reads each request into `$line`, `$id` and `$wav`, counting them in
    /// `$n`: the first one is the prewarm.
    const READ_REQUESTS: &str = r#"n=0
while read line; do
n=$((n + 1))
id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
wav=$(echo "$line" | sed 's/.*"wav":"\([^"]*\)".*/\1/')"#;

    const ANSWER: &str = r#"echo "{\"type\":\"text\",\"id\":$id,\"text\":\"\"}""#;

    #[tokio::test]
    async fn prewarm_request_is_sent_after_ready() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("requests");
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
echo "$n $(wc -c < "$wav" | tr -d ' ') $line" >> "{}"
{ANSWER}
done"#,
                log.display()
            ),
        );
        let mut model = runtime.load().await.unwrap();
        let requests = std::fs::read_to_string(&log).unwrap();
        // One second of silence: 44 bytes of header, then 16-bit samples.
        let silence = format!("1 {} ", 44 + 2 * SAMPLES_PER_SECOND);
        assert!(requests.starts_with(&silence), "{requests}");
        assert_eq!(requests.lines().count(), 1, "{requests}");
        assert!(!requests.contains("terms") && !requests.contains("background"));
        assert!(!requests.contains("language"), "{requests}");

        model.transcribe(&samples(), &[], None).await.unwrap();
        let requests = std::fs::read_to_string(&log).unwrap();
        assert_eq!(requests.lines().count(), 2, "{requests}");
    }

    #[tokio::test]
    async fn prewarm_failure_drops_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken-once");
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
if [ ! -e "{broken}" ]; then
touch "{broken}"
echo "{{\"type\":\"error\",\"id\":$id,\"stage\":\"transcribe\",\"code\":\"io\",\"message\":\"broken model\"}}"
else
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"ok $n\"}}"
fi
done"#,
                broken = broken.display()
            ),
        );
        let session = Session::new(Box::new(runtime), Box::new(|| false));
        let error = session.warm().await.unwrap_err();
        assert!(error.to_string().contains("broken model"), "{error}");
        assert!(!session.is_loaded().await);

        // The next dictation starts a fresh helper, prewarmed again.
        let text = session
            .transcribe(&samples(), &[], None, Duration::from_secs(30))
            .await
            .unwrap();
        assert_eq!(text.text, "ok 2");
    }

    #[tokio::test]
    async fn warm_waits_for_optimizing_without_cloud() {
        let without_cloud = Conditions {
            model: LocalReadiness::Optimizing,
            signed_in: false,
            language: "fr",
        };
        assert_eq!(
            route(Engine::Local, without_cloud),
            Route::Run(EngineUsed {
                engine: Engine::Local,
                fallback: None,
            })
        );

        let dir = tempfile::tempdir().unwrap();
        let started = dir.path().join("started");
        let (state, receiver) = watch::channel(LocalModelState::Optimizing);
        let runtime = runtime_while(
            dir.path(),
            &format!(
                r#"touch "{}"
echo '{{"type":"ready"}}'
{READ_REQUESTS}
{ANSWER}
done"#,
                started.display()
            ),
            receiver,
        );
        let session = Session::new(Box::new(runtime), Box::new(|| false));
        let warming = tokio::spawn({
            let session = Arc::clone(&session);
            async move { session.warm().await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!warming.is_finished(), "loaded during the compile");
        assert!(!started.exists(), "a second compile started beside it");

        state.send_replace(LocalModelState::Ready);
        warming.await.unwrap().unwrap();
        assert!(started.exists());
        assert!(session.is_loaded().await);
    }

    fn samples() -> Vec<f32> {
        vec![0.1; 1_600]
    }

    #[tokio::test]
    async fn each_piece_is_answered_by_id_and_its_wav_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo 'loading'
echo '{{"type":"progress","phase":"compile","fraction":1}}'
echo '{{"type":"ready"}}'
{READ_REQUESTS}
size=$(wc -c < "$wav" | tr -d ' ')
echo '{{"type":"text","id":999,"text":"stale"}}'
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"Bonjour $size\"}}"
done"#
            ),
        );
        let mut model = runtime.load().await.unwrap();
        // 44 bytes of header, then 16-bit samples.
        let expected = format!("Bonjour {}", 44 + 2 * samples().len());
        for _ in 0..2 {
            let heard = model.transcribe(&samples(), &[], None).await.unwrap();
            assert_eq!(heard.text, expected);
        }
        let left: Vec<_> = std::fs::read_dir(dir.path().join("scratch"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[tokio::test]
    async fn the_terms_reach_the_helper_only_when_there_are_some() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
case "$line" in
*'"terms":["Siobhán","Nguyen"]'*) heard=boosted ;;
*terms*) heard=garbled ;;
*) heard=plain ;;
esac
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"$heard\"}}"
done"#
            ),
        );
        let mut model = runtime.load().await.unwrap();
        let terms = ["Siobhán".to_string(), "Nguyen".to_string()];
        let heard = model.transcribe(&samples(), &terms, None).await.unwrap();
        assert_eq!(heard.text, "boosted");
        let heard = model.transcribe(&samples(), &[], None).await.unwrap();
        assert_eq!(heard.text, "plain");
    }

    #[tokio::test]
    async fn the_language_reaches_the_helper_only_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
case "$line" in
*'"language":"fr"'*) heard=french ;;
*language*) heard=garbled ;;
*) heard=detected ;;
esac
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"$heard\"}}"
done"#
            ),
        );
        let mut model = runtime.load().await.unwrap();
        let heard = model.transcribe(&samples(), &[], Some("fr")).await.unwrap();
        assert_eq!(heard.text, "french");
        let heard = model.transcribe(&samples(), &[], None).await.unwrap();
        assert_eq!(heard.text, "detected");
    }

    #[tokio::test]
    async fn the_words_of_a_reply_come_with_their_times() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"Bonjour à tous.\",\"words\":[{{\"w\":\"Bonjour\",\"s\":0.25,\"e\":0.75}},{{\"w\":\"à\",\"s\":0.75,\"e\":1.0}},{{\"w\":\"tous.\",\"s\":1.0,\"e\":1.5}}]}}"
done"#
            ),
        );
        let mut model = runtime.load().await.unwrap();
        let heard = model.transcribe(&samples(), &[], None).await.unwrap();
        assert_eq!(
            heard,
            Heard {
                text: "Bonjour à tous.".into(),
                words: vec![0.25..0.75, 0.75..1.0, 1.0..1.5],
            }
        );
    }

    #[tokio::test]
    async fn a_failed_piece_is_an_error_and_the_helper_keeps_serving() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
if [ $n -eq 2 ]; then
echo "{{\"type\":\"error\",\"id\":$id,\"stage\":\"transcribe\",\"code\":\"io\",\"message\":\"bad audio\"}}"
else
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"ok\"}}"
fi
done"#
            ),
        );
        let mut model = runtime.load().await.unwrap();
        let error = model.transcribe(&samples(), &[], None).await.unwrap_err();
        assert!(error.to_string().contains("bad audio"), "{error}");
        let heard = model.transcribe(&samples(), &[], None).await.unwrap();
        assert_eq!(heard.text, "ok");
    }

    #[tokio::test]
    async fn a_helper_that_cannot_load_or_dies_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = runtime(
            dir.path(),
            r#"echo '{"type":"error","stage":"load","code":"model_missing","message":"no models"}'
exit 1"#,
        );
        let error = missing.load().await.err().unwrap();
        assert!(error.to_string().contains("ModelMissing"), "{error}");

        let crashes = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
[ $n -eq 2 ] && kill -9 $$
{ANSWER}
done"#
            ),
        );
        let mut model = crashes.load().await.unwrap();
        let error = model.transcribe(&samples(), &[], None).await.unwrap_err();
        assert!(error.to_string().contains("stopped"), "{error}");
    }
}
