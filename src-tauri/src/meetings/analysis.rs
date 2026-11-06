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
use super::protocol::{timed_words, ErrorCode, Message, Turn, Word};
use super::store::SpeakerAnalysis;
use crate::local_models::manifest::Asr;
use crate::ProcessingOperation;

pub const ANALYSIS_FILE: &str = "analysis.json";
/// A 3-hour meeting takes a few minutes; this only catches a hung helper.
#[cfg_attr(target_os = "windows", allow(dead_code))]
const ANALYZE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// System audio where this many words were heard holds the remote side of
/// a call, however long the user spoke. A ringtone or a chime gives next to
/// none; 45 seconds of questions give about a hundred.
const REMOTE_WORDS: usize = 15;

/// The track whose voices are told apart, from the words heard on each:
/// system audio when the remote side of a call spoke there, that much or
/// more than the microphone, else the microphone, where every voice of a
/// meeting held in a room is. Without the words of both tracks, `by_levels`
/// decides.
pub fn diarized_track(words: &BTreeMap<Track, Vec<Word>>, by_levels: Track) -> Track {
    let heard = |track| words.get(&track).map(|words| timed_words(words).len());
    match (heard(Track::Mic), heard(Track::System)) {
        (Some(mic), Some(system)) if system >= REMOTE_WORDS || system > mic => Track::System,
        (Some(_), Some(_)) => Track::Mic,
        _ => by_levels,
    }
}

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

    /// How the speakers can be told apart with this analysis. Turns that
    /// name nobody did not tell anyone apart.
    pub fn outcome(&self) -> SpeakerAnalysis {
        if self.turns.as_ref().is_some_and(|turns| !turns.is_empty()) {
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
    /// The track whose voices are told apart when the words heard on device
    /// cannot say (`diarized_track`).
    pub by_levels: Track,
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

/// The analyzer this platform runs: the helper process on a Mac, the models
/// in the app on Windows (`analysis_cpp`).
pub fn platform_analyzer(models: Arc<Models>) -> Arc<dyn Analyzer> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Arc::new(super::analysis_cpp::CppAnalyzer { models })
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        Arc::new(HelperAnalyzer {
            models,
            support: helper::support(),
        })
    }
}

/// Runs the analysis in `murmur-meetings-helper` with the downloaded models.
/// Windows has no helper: it runs the models in the app (`analysis_cpp`).
#[cfg_attr(target_os = "windows", allow(dead_code))]
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
            let Some(program) = helper::helper_path() else {
                log::error!("The meetings helper is missing from the app");
                return AnalyzerOutcome::Ran(Analysis::default());
            };
            let started = std::time::Instant::now();
            let (mut analysis, ended, asr) =
                analyze_on_the_model_in_use(&program, &models, &request, &operation).await;
            match ended {
                Some(Ended::Done) => analysis.complete = true,
                Some(Ended::Cancelled) => return AnalyzerOutcome::Cancelled,
                Some(ended) => log::warn!("Meeting analysis ended early: {ended:?}"),
                None => {}
            }
            if analysis.models_missing {
                // Run again once the models are back.
                analysis.complete = false;
                models.invalidate(asr);
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

/// `analyze_and_diarize` with the speech model the folder names, and the
/// model it ran. When the switch to the current model deleted the previous
/// one before the helper opened it, it runs again on the current one
/// (`runs_again`).
#[cfg_attr(target_os = "windows", allow(dead_code))]
async fn analyze_on_the_model_in_use(
    program: &Path,
    models: &Models,
    request: &AnalysisRequest,
    operation: &ProcessingOperation,
) -> (Analysis, Option<Ended>, Asr) {
    let asr = crate::local_models::asr_in_use(models.dir());
    let (analysis, ended) = analyze_and_diarize(program, models, asr, request, operation).await;
    let now = crate::local_models::asr_in_use(models.dir());
    if !runs_again(
        ended,
        analysis.models_missing,
        asr,
        now,
        operation.is_cancelled(),
    ) {
        return (analysis, ended, asr);
    }
    let (analysis, ended) = analyze_and_diarize(program, models, now, request, operation).await;
    (analysis, ended, now)
}

/// Whether a run on `ran` goes again on `now`: only when the helper stopped
/// by itself after finding the models missing, the folder names another
/// model since, and the analysis was not cancelled. A cancel, a timeout or a
/// helper that could not start ends the analysis there.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn runs_again(
    ended: Option<Ended>,
    models_missing: bool,
    ran: Asr,
    now: Asr,
    cancelled: bool,
) -> bool {
    matches!(ended, Some(Ended::Done | Ended::Crashed))
        && models_missing
        && ran != now
        && !cancelled
}

/// The words first: what was said on each track picks the one whose voices
/// are told apart.
#[cfg_attr(target_os = "windows", allow(dead_code))]
async fn analyze_and_diarize(
    program: &Path,
    models: &Models,
    asr: Asr,
    request: &AnalysisRequest,
    operation: &ProcessingOperation,
) -> (Analysis, Option<Ended>) {
    let mut analysis = Analysis::default();
    let mut ended = run_helper(
        program,
        "analyze",
        models,
        asr,
        &request.tracks,
        operation,
        &mut analysis,
    )
    .await;
    if ended == Some(Ended::Done) && !analysis.models_missing {
        let diarize = diarized_track(&analysis.words, request.by_levels);
        analysis.diarized = Some(diarize);
        let track: Vec<(Track, PathBuf)> = request
            .tracks
            .iter()
            .filter(|(track, _)| *track == diarize)
            .cloned()
            .collect();
        ended = run_helper(
            program,
            "diarize",
            models,
            asr,
            &track,
            operation,
            &mut analysis,
        )
        .await;
    }
    (analysis, ended)
}

/// Runs one helper `command` on `tracks`, adding what it prints to
/// `analysis`. `None` when the helper cannot be run.
#[cfg_attr(target_os = "windows", allow(dead_code))]
async fn run_helper(
    program: &Path,
    command: &str,
    models: &Models,
    asr: Asr,
    tracks: &[(Track, PathBuf)],
    operation: &ProcessingOperation,
    analysis: &mut Analysis,
) -> Option<Ended> {
    let mut args: Vec<OsString> = vec![
        command.into(),
        "--models-dir".into(),
        models.dir().as_os_str().to_owned(),
        "--asr".into(),
        asr.helper_name().into(),
    ];
    for (track, path) in tracks {
        args.push(format!("--{}", track_arg(*track)).into());
        args.push(path.clone().into_os_string());
    }
    helper::run(program, &args, operation, ANALYZE_TIMEOUT, |message| {
        analysis.absorb(message)
    })
    .await
    .inspect_err(|error| log::error!("Cannot run the meetings helper: {error}"))
    .ok()
}

#[cfg_attr(target_os = "windows", allow(dead_code))]
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

    #[cfg(unix)]
    struct NoHost;

    #[cfg(unix)]
    impl crate::local_models::Host for NoHost {
        fn notify(&self, _: crate::local_models::LocalModelStatus) {}
        fn remember_wanted(&self, _: bool) {}
        fn free_bytes(&self, _: &Path) -> Option<u64> {
            None
        }
        fn prepare(
            &self,
            _: PathBuf,
            _: ProcessingOperation,
        ) -> crate::local_models::PrepareFuture {
            Box::pin(async { crate::local_models::Prepared::Done })
        }
    }

    /// A models folder ready with v3, and a stand-in helper that logs each
    /// run as `<command> <asr>` to `runs`. Asked for v3, it switches the
    /// folder to Ultra, reports the models missing and exits, as the helper
    /// does.
    #[cfg(unix)]
    fn switching_helper(dir: &Path) -> (Arc<Models>, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("models");
        std::fs::create_dir_all(&path).unwrap();
        let marker = path.join(".murmur-ready");
        std::fs::write(&marker, crate::local_models::manifest::PREVIOUS.ready_id).unwrap();
        let models = Models::new(
            path,
            crate::local_models::manifest::this_mac(),
            AnalysisSupport::Available,
            "http://127.0.0.1:9".to_string(),
            Arc::new(NoHost),
        );
        let log = dir.join("runs");
        let program = dir.join("helper.sh");
        std::fs::write(
            &program,
            format!(
                r#"#!/bin/sh
echo "$1 $5" >> "{log}"
if [ "$5" = v3 ]; then
printf ultra > "{marker}"
echo '{{"type":"error","stage":"load","code":"model_missing","message":"no v3"}}'
exit 1
fi
if [ "$1" = analyze ]; then
echo '{{"type":"words","track":"mic","words":[{{"w":"oui","s":1,"e":1.2}}]}}'
else
echo '{{"type":"diarization","track":"mic","segments":[{{"spk":"A","s":0,"e":2}}]}}'
fi
echo '{{"type":"done"}}'"#,
                log = log.display(),
                marker = marker.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        (models, program, log)
    }

    #[cfg(unix)]
    fn mic_request(dir: &Path) -> AnalysisRequest {
        AnalysisRequest {
            tracks: vec![(Track::Mic, dir.join("mic.wav"))],
            by_levels: Track::Mic,
        }
    }

    /// The switch to Ultra deletes v3 before the helper opens it: the
    /// analysis runs again on Ultra instead of waiting for models that are
    /// not missing.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_analysis_that_lost_v3_to_the_switch_runs_again_on_ultra() {
        let dir = tempfile::tempdir().unwrap();
        let (models, program, log) = switching_helper(dir.path());
        let request = mic_request(dir.path());

        let (analysis, ended, asr) = analyze_on_the_model_in_use(
            &program,
            &models,
            &request,
            &ProcessingOperation::new(crate::OperationId(1)),
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "analyze v3\nanalyze ultra\ndiarize ultra\n"
        );
        assert_eq!(ended, Some(Ended::Done));
        assert_eq!(asr, Asr::Current);
        assert!(!analysis.models_missing);
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Separated);
    }

    #[test]
    fn only_a_helper_that_stopped_by_itself_on_missing_v3_runs_again() {
        let ends = [
            None,
            Some(Ended::Done),
            Some(Ended::Crashed),
            Some(Ended::Cancelled),
            Some(Ended::TimedOut),
        ];
        for ended in ends {
            for missing in [false, true] {
                for now in [Asr::Previous, Asr::Current] {
                    for cancelled in [false, true] {
                        let expected = [
                            (Some(Ended::Done), true, Asr::Current, false),
                            (Some(Ended::Crashed), true, Asr::Current, false),
                        ]
                        .contains(&(ended, missing, now, cancelled));
                        assert_eq!(
                            runs_again(ended, missing, Asr::Previous, now, cancelled),
                            expected,
                            "{ended:?} missing={missing} now={now:?} cancelled={cancelled}"
                        );
                    }
                }
            }
        }
        assert!(!runs_again(
            Some(Ended::Crashed),
            true,
            Asr::Current,
            Asr::Current,
            false
        ));
    }

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

        // Silence: nobody to tell apart, so the user may try again.
        analysis.absorb(
            parse_line(r#"{"type":"diarization","track":"system","segments":[]}"#).unwrap(),
        );
        assert_eq!(analysis.outcome(), SpeakerAnalysis::Failed);
        analysis.absorb(
            parse_line(
                r#"{"type":"diarization","track":"system","segments":[{"spk":"X","s":0,"e":1}]}"#,
            )
            .unwrap(),
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

    fn heard(text: &str, count: usize) -> Vec<Word> {
        (0..count)
            .map(|i| Word {
                w: text.into(),
                s: i as f64,
                e: i as f64 + 0.5,
            })
            .collect()
    }

    #[test]
    fn the_words_heard_on_system_audio_pick_the_track_whose_voices_are_told_apart() {
        // An hour of presenting, then 45 seconds of remote questions.
        let call = BTreeMap::from([
            (Track::Mic, heard("mot", 7_000)),
            (Track::System, heard("question", 100)),
        ]);
        assert_eq!(diarized_track(&call, Track::Mic), Track::System);
        // A short call where the user stayed muted.
        let muted = BTreeMap::from([(Track::Mic, Vec::new()), (Track::System, heard("oui", 5))]);
        assert_eq!(diarized_track(&muted, Track::Mic), Track::System);
        // A ringtone or a chime in a room: louder than the room, maybe, but
        // next to nothing was said there.
        for system in [Vec::new(), heard("ding", 2), heard("…", 40)] {
            let room = BTreeMap::from([(Track::Mic, heard("mot", 700)), (Track::System, system)]);
            assert_eq!(diarized_track(&room, Track::System), Track::Mic);
        }
        // A track that was not heard leaves the choice to the levels.
        let system_only = BTreeMap::from([(Track::System, heard("ding", 2))]);
        assert_eq!(diarized_track(&system_only, Track::System), Track::System);
        assert_eq!(diarized_track(&BTreeMap::new(), Track::Mic), Track::Mic);
    }
}
