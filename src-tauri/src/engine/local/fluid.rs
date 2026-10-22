//! Apple silicon: the meetings helper's `serve` mode keeps FluidAudio's
//! Parakeet loaded and answers one request per line. Each piece crosses as a
//! WAV in the app's cache, deleted once answered. The helper stays a separate
//! process so a Core ML crash cannot take the app down, and killing it is how
//! its memory is released.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::boost::BoostTerms;
use super::session::{BoxFuture, Loaded, Runtime};
use crate::meetings::protocol::{parse_reply, Reply, Request};

/// A load from compiled models takes about 2 s; a first one after a macOS
/// update compiles them again, about 15 s.
const LOAD_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Unique across the dictation and meeting sessions, which share the folder.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct FluidRuntime {
    program: PathBuf,
    models: PathBuf,
    scratch: PathBuf,
}

impl FluidRuntime {
    pub fn new(program: PathBuf, models: PathBuf, scratch: PathBuf) -> Self {
        Self {
            program,
            models,
            scratch,
        }
    }
}

impl Runtime for FluidRuntime {
    fn load(&self) -> BoxFuture<'static, anyhow::Result<Box<dyn Loaded>>> {
        let (program, models, scratch) = (
            self.program.clone(),
            self.models.clone(),
            self.scratch.clone(),
        );
        Box::pin(async move {
            std::fs::create_dir_all(&scratch)?;
            let mut serve = Serve::spawn(&program, &models, scratch)?;
            match tokio::time::timeout(LOAD_TIMEOUT, serve.ready()).await {
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
        let mut child = Command::new(program)
            .arg("serve")
            .arg("--models-dir")
            .arg(models)
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
    /// The helper boosts `terms` with FluidAudio's CTC keyword spotting when
    /// the optional CTC model is downloaded, and otherwise ignores them.
    fn transcribe<'a>(
        &'a mut self,
        samples: &'a [f32],
        terms: &'a BoostTerms,
    ) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let wav = Scratch(self.scratch.join(format!("piece-{id}.wav")));
            crate::audio_toolkit::save_wav_file(&wav.0, samples)?;
            let mut line = serde_json::to_string(&Request::Transcribe {
                id,
                wav: &wav.0,
                terms: &terms.words,
                names: &terms.names,
            })?;
            line.push('\n');
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await?;
            // Answers to an abandoned request may come first.
            loop {
                match self.reply().await? {
                    Reply::Text { id: answered, text } if answered == id => return Ok(text),
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    static NONE: BoostTerms = BoostTerms {
        words: Vec::new(),
        names: Vec::new(),
    };

    /// A stand-in helper: a shell script with the given body. `$wav` and
    /// `$id` hold the fields of each request it reads.
    fn runtime(dir: &Path, body: &str) -> FluidRuntime {
        let program = dir.join("helper.sh");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        FluidRuntime::new(program, dir.join("models"), dir.join("scratch"))
    }

    const READ_REQUESTS: &str = r#"while read line; do
id=$(echo "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
wav=$(echo "$line" | sed 's/.*"wav":"\([^"]*\)".*/\1/')"#;

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
        assert_eq!(model.transcribe(&samples(), &NONE).await.unwrap(), expected);
        assert_eq!(model.transcribe(&samples(), &NONE).await.unwrap(), expected);
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
*'"terms":["Siobhán"],"names":["Nguyen"]'*) heard=boosted ;;
*terms*|*names*) heard=garbled ;;
*) heard=plain ;;
esac
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"$heard\"}}"
done"#
            ),
        );
        let mut model = runtime.load().await.unwrap();
        let terms = BoostTerms {
            words: vec!["Siobhán".to_string()],
            names: vec!["Nguyen".to_string()],
        };
        assert_eq!(
            model.transcribe(&samples(), &terms).await.unwrap(),
            "boosted"
        );
        assert_eq!(model.transcribe(&samples(), &NONE).await.unwrap(), "plain");
    }

    #[tokio::test]
    async fn a_failed_piece_is_an_error_and_the_helper_keeps_serving() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = runtime(
            dir.path(),
            &format!(
                r#"echo '{{"type":"ready"}}'
{READ_REQUESTS}
marker="$(dirname "$wav")/failed-once"
if [ -e "$marker" ]; then
echo "{{\"type\":\"text\",\"id\":$id,\"text\":\"ok\"}}"
else
touch "$marker"
echo "{{\"type\":\"error\",\"id\":$id,\"stage\":\"transcribe\",\"code\":\"io\",\"message\":\"bad audio\"}}"
fi
done"#
            ),
        );
        let mut model = runtime.load().await.unwrap();
        let error = model.transcribe(&samples(), &NONE).await.unwrap_err();
        assert!(error.to_string().contains("bad audio"), "{error}");
        assert_eq!(model.transcribe(&samples(), &NONE).await.unwrap(), "ok");
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
kill -9 $$
done"#
            ),
        );
        let mut model = crashes.load().await.unwrap();
        let error = model.transcribe(&samples(), &NONE).await.unwrap_err();
        assert!(error.to_string().contains("stopped"), "{error}");
    }
}
