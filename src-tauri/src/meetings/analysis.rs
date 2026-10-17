//! The on-device analysis of a meeting: the words the local recogniser heard
//! on each track, with their times, and who spoke when on one track. It is
//! saved as `analysis.json` next to the tracks, so the transcript can be
//! rebuilt without running the models again.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use super::helper::{self, AnalysisSupport, Ended};
use super::merge::Track;
use super::models::Models;
use super::protocol::{ErrorCode, Message, Turn, Word};
use super::store::SpeakerAnalysis;
use crate::ProcessingOperation;

pub const ANALYSIS_FILE: &str = "analysis.json";
/// A 3-hour meeting takes a few minutes; this only catches a hung helper.
const ANALYZE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Analysis {
    pub words: BTreeMap<Track, Vec<Word>>,
    /// The track whose speakers were told apart, and its turns.
    pub diarized: Option<Track>,
    pub turns: Option<Vec<Turn>>,
    /// The helper finished: running it again would give the same result.
    pub complete: bool,
    /// The helper found the models missing.
    #[serde(default)]
    pub models_missing: bool,
}

impl Analysis {
    pub fn absorb(&mut self, message: Message) {
        match message {
            Message::Words { track, words } => {
                self.words.insert(track, words);
            }
            Message::Diarization { track, segments } if Some(track) == self.diarized => {
                self.turns = Some(segments);
            }
            Message::Error { stage, code, .. } => {
                log::warn!("Meeting analysis: {stage} failed ({code:?})");
                if code == ErrorCode::ModelMissing {
                    self.models_missing = true;
                }
            }
            _ => {}
        }
    }

    /// How the speakers can be told apart with this analysis.
    pub fn outcome(&self) -> SpeakerAnalysis {
        if self.turns.is_some() {
            SpeakerAnalysis::Separated
        } else if self.models_missing {
            SpeakerAnalysis::NoModels
        } else {
            SpeakerAnalysis::Failed
        }
    }

    /// A finished analysis saved earlier, if any.
    pub fn load(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(ANALYSIS_FILE)).ok()?;
        serde_json::from_str::<Self>(&text)
            .inspect_err(|error| log::warn!("Ignoring an unreadable meeting analysis: {error}"))
            .ok()
            .filter(|analysis| analysis.complete)
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        let path = dir.join(ANALYSIS_FILE);
        let partial = dir.join(format!("{ANALYSIS_FILE}.partial"));
        std::fs::write(&partial, serde_json::to_vec(self)?)?;
        std::fs::rename(partial, path)?;
        Ok(())
    }
}

pub struct AnalysisRequest {
    pub tracks: Vec<(Track, PathBuf)>,
    pub diarize: Track,
}

pub enum AnalyzerOutcome {
    /// Nothing ran: one speaker per track.
    Unavailable(SpeakerAnalysis),
    Ran(Analysis),
    Cancelled,
}

pub type AnalyzeFuture = Pin<Box<dyn Future<Output = AnalyzerOutcome> + Send>>;

pub trait Analyzer: Send + Sync + 'static {
    fn analyze(&self, request: AnalysisRequest, operation: ProcessingOperation) -> AnalyzeFuture;
}

/// Runs the analysis in `murmur-meetings-helper` with the downloaded models.
pub struct HelperAnalyzer {
    pub models: Arc<Models>,
    pub support: AnalysisSupport,
}

impl Analyzer for HelperAnalyzer {
    fn analyze(&self, request: AnalysisRequest, operation: ProcessingOperation) -> AnalyzeFuture {
        let models = Arc::clone(&self.models);
        let support = self.support;
        Box::pin(async move {
            if support != AnalysisSupport::Available {
                return AnalyzerOutcome::Unavailable(SpeakerAnalysis::Unsupported);
            }
            // A download started with the meeting may still be finishing.
            if !models.wait_ready(&operation).await {
                return if operation.is_cancelled() {
                    AnalyzerOutcome::Cancelled
                } else {
                    AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels)
                };
            }
            let mut analysis = Analysis {
                diarized: Some(request.diarize),
                ..Analysis::default()
            };
            let Some(program) = helper::helper_path() else {
                log::error!("The meetings helper is missing from the app");
                return AnalyzerOutcome::Ran(analysis);
            };
            let mut args: Vec<OsString> = vec![
                "analyze".into(),
                "--models-dir".into(),
                models.dir().as_os_str().to_owned(),
            ];
            for (track, path) in &request.tracks {
                args.push(format!("--{}", track_arg(*track)).into());
                args.push(path.clone().into_os_string());
            }
            args.push("--diarize".into());
            args.push(track_arg(request.diarize).into());
            let started = std::time::Instant::now();
            let ended = helper::run(&program, &args, &operation, ANALYZE_TIMEOUT, |message| {
                analysis.absorb(message)
            })
            .await;
            match ended {
                Ok(Ended::Done) => analysis.complete = true,
                Ok(Ended::Cancelled) => return AnalyzerOutcome::Cancelled,
                Ok(ended) => log::warn!("Meeting analysis ended early: {ended:?}"),
                Err(error) => log::error!("Cannot run the meetings helper: {error}"),
            }
            if analysis.models_missing {
                // Run again once the models are back.
                analysis.complete = false;
                models.invalidate();
            }
            log::info!(
                "Meeting analysis: {:?} in {:.1} s",
                analysis.outcome(),
                started.elapsed().as_secs_f32()
            );
            AnalyzerOutcome::Ran(analysis)
        })
    }
}

fn track_arg(track: Track) -> &'static str {
    match track {
        Track::Mic => "mic",
        Track::System => "system",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::protocol::parse_line;

    #[test]
    fn helper_messages_fill_the_analysis() {
        let mut analysis = Analysis {
            diarized: Some(Track::System),
            ..Analysis::default()
        };
        for line in [
            r#"{"type":"words","track":"mic","words":[{"w":"oui","s":1,"e":1.2}]}"#,
            r#"{"type":"diarization","track":"mic","segments":[{"spk":"X","s":0,"e":1}]}"#,
            r#"{"type":"error","stage":"diarize","code":"crash","message":"m"}"#,
        ] {
            analysis.absorb(parse_line(line).unwrap());
        }
        assert_eq!(analysis.words[&Track::Mic].len(), 1);
        // Turns of a track that was not asked for are ignored.
        assert_eq!(analysis.turns, None);
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Failed);

        analysis.absorb(
            parse_line(r#"{"type":"diarization","track":"system","segments":[]}"#).unwrap(),
        );
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Separated);

        let mut missing = Analysis::default();
        missing.absorb(
            parse_line(r#"{"type":"error","stage":"asr","code":"model_missing"}"#).unwrap(),
        );
        assert_eq!(missing.outcome(), SpeakerAnalysis::NoModels);
    }

    #[test]
    fn only_a_finished_analysis_is_reused() {
        let dir = tempfile::tempdir().unwrap();
        let mut analysis = Analysis {
            diarized: Some(Track::System),
            turns: Some(vec![]),
            ..Analysis::default()
        };
        analysis.save(dir.path()).unwrap();
        assert_eq!(Analysis::load(dir.path()), None);
        analysis.complete = true;
        analysis.save(dir.path()).unwrap();
        assert_eq!(Analysis::load(dir.path()), Some(analysis));
        std::fs::write(dir.path().join(ANALYSIS_FILE), "{").unwrap();
        assert_eq!(Analysis::load(dir.path()), None);
    }
}
