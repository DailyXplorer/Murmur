//! The live transcript of the meeting being recorded: a draft, a few seconds
//! behind speech, made on this Mac by the dictation's own local model at
//! background priority, so a dictation always goes first. Each track's
//! writer hands its audio to a `LiveTap`, which cuts it into pieces
//! (`chunker`); the `worker` transcribes them one at a time, joins them into
//! sentences (`seam`), hides the microphone's echo of the remote side
//! (`echo`) and appends them to `live.ndjson` in the meeting's folder
//! (`draft`). Never through the cloud. The final transcript, made after Stop
//! as before, replaces the draft, which is then deleted.

pub mod chunker;
pub mod draft;
pub mod echo;
pub mod paste;
pub mod seam;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::mpsc;

use super::merge::Track as Source;
use chunker::{Chunk, Chunker, LiveBlock};
pub use worker::LiveEngine;

/// "Paste new meeting text", unbound until the user picks keys.
pub const PASTE_BINDING_ID: &str = "meeting_paste_new";

const QUEUE_CHUNKS: usize = 64;
const FIRST_PASTE: Duration = Duration::from_secs(60);

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum LiveEntryKind {
    Speech,
    /// Audio the live text skipped; the final transcript has it.
    Gap,
}

/// One sentence of the draft, on the meeting timeline.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Type)]
pub struct LiveEntry {
    pub source: Source,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub kind: LiveEntryKind,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum LiveUnavailable {
    /// The live transcript setting is off.
    Off,
    Missing,
    Downloading,
    Preparing,
    /// An Intel Mac.
    Unsupported,
    /// Three pieces in a row failed.
    EngineFailed,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LiveState {
    /// Recording, nothing said yet.
    Starting,
    Live,
    /// Over 30 s of speech wait for the model.
    CatchingUp,
    Unavailable {
        reason: LiveUnavailable,
    },
    /// The meeting stopped: the draft waits for the final transcript.
    Stopped,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct LiveTranscript {
    pub state: LiveState,
    /// In the order they were published, which is not quite time order.
    pub entries: Vec<LiveEntry>,
}

/// Sent as the live transcript of meeting `id` grows or changes state.
#[derive(Serialize, Debug, Clone, Type, tauri_specta::Event)]
pub struct MeetingLiveUpdated {
    pub id: i64,
    pub state: LiveState,
    pub appended: Vec<LiveEntry>,
}

struct Current {
    id: i64,
    origin: Instant,
    transcript: LiveTranscript,
    pasted: Option<usize>,
}

/// The live transcript of the last meeting recorded since launch, which
/// the view and the paste shortcut read.
pub struct Live {
    current: Mutex<Option<Current>>,
    emit: Box<dyn Fn(MeetingLiveUpdated) + Send + Sync>,
}

impl Live {
    pub fn new(emit: Box<dyn Fn(MeetingLiveUpdated) + Send + Sync>) -> Self {
        Self {
            current: Mutex::new(None),
            emit,
        }
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<Current>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn begin(&self, id: i64, state: LiveState) {
        *self.current() = Some(Current {
            id,
            origin: Instant::now(),
            transcript: LiveTranscript {
                state,
                entries: Vec::new(),
            },
            pasted: None,
        });
        (self.emit)(MeetingLiveUpdated {
            id,
            state,
            appended: Vec::new(),
        });
    }

    fn update(&self, id: i64, state: LiveState, appended: Vec<LiveEntry>) {
        {
            let mut current = self.current();
            let Some(current) = current.as_mut().filter(|current| current.id == id) else {
                return;
            };
            if current.transcript.state == state && appended.is_empty() {
                return;
            }
            current.transcript.state = state;
            current.transcript.entries.extend(appended.iter().cloned());
        }
        (self.emit)(MeetingLiveUpdated {
            id,
            state,
            appended,
        });
    }

    pub fn forget(&self, id: i64) {
        let mut current = self.current();
        if current.as_ref().is_some_and(|current| current.id == id) {
            *current = None;
        }
    }

    fn transcript(&self, id: i64) -> Option<LiveTranscript> {
        self.current()
            .as_ref()
            .filter(|current| current.id == id)
            .map(|current| current.transcript.clone())
    }

    /// The sentences said since the last paste, or in the last minute at the
    /// first one, in time order. Empty when nothing new was said.
    pub fn take_new(&self) -> Vec<LiveEntry> {
        let mut current = self.current();
        let Some(current) = current.as_mut() else {
            return Vec::new();
        };
        let entries = &current.transcript.entries;
        let mut new: Vec<LiveEntry> = match current.pasted {
            Some(pasted) => entries[pasted..].to_vec(),
            None => {
                let since = (current.origin.elapsed().saturating_sub(FIRST_PASTE)).as_millis();
                entries
                    .iter()
                    .filter(|entry| u128::from(entry.end_ms) >= since)
                    .cloned()
                    .collect()
            }
        };
        current.pasted = Some(entries.len());
        new.retain(|entry| entry.kind == LiveEntryKind::Speech);
        new.sort_by_key(|entry| entry.start_ms);
        new
    }
}

/// The live transcript of meeting `id`: the one recording or just recorded,
/// else the draft a crash or a quit left in its folder, until the final
/// transcript replaced it.
pub fn transcript_of(live: &Live, meeting_dir: &Path, id: i64) -> Option<LiveTranscript> {
    let entries = draft::read(meeting_dir);
    match live.transcript(id) {
        Some(transcript) if transcript.state != LiveState::Stopped || entries.is_some() => {
            Some(transcript)
        }
        Some(_) => None,
        None => Some(LiveTranscript {
            state: LiveState::Stopped,
            entries: entries?,
        }),
    }
}

/// `You: …` and `Others: …` lines, labelled in the app's language.
pub fn paste_text(entries: &[LiveEntry], you: &str, others: &str) -> String {
    entries
        .iter()
        .map(|entry| {
            let label = match entry.source {
                Source::Mic => you,
                Source::System => others,
            };
            format!("{label}: {}", entry.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Handed to a track's writer: cuts what it writes into pieces and queues
/// them. It never blocks the writer; a full queue drops the piece.
pub struct LiveTap {
    chunker: Chunker,
    sender: mpsc::Sender<Chunk>,
    dropped: usize,
}

impl LiveTap {
    /// Audio the writer placed at `start_sample` on the meeting timeline.
    pub fn push(&mut self, start_sample: u64, samples: &[f32]) {
        for chunk in self.chunker.push(LiveBlock {
            start_sample,
            samples,
        }) {
            self.send(chunk);
        }
    }

    /// The track ended: its last piece goes too.
    pub fn finish(mut self) {
        if let Some(chunk) = self.chunker.finish() {
            self.send(chunk);
        }
        if self.dropped > 0 {
            log::warn!("The live transcript dropped {} piece(s)", self.dropped);
        }
    }

    fn send(&mut self, chunk: Chunk) {
        if self.sender.try_send(chunk).is_err() {
            self.dropped += 1;
        }
    }
}

/// The live transcript of the meeting being recorded, held by the
/// coordinator. Dropping it, once the tracks finished, lets the worker
/// publish what is left and mark the draft stopped.
pub struct LiveMeeting {
    sender: mpsc::Sender<Chunk>,
    headphones: Arc<AtomicBool>,
    discarded: Arc<AtomicBool>,
}

impl LiveMeeting {
    pub fn tap(&self, source: Source) -> LiveTap {
        LiveTap {
            chunker: Chunker::new(source),
            sender: self.sender.clone(),
            dropped: 0,
        }
    }

    /// On headphones the microphone hears no one else, so nothing is held
    /// back as a possible echo.
    pub fn set_headphones(&self, on: bool) {
        self.headphones.store(on, Ordering::Relaxed);
    }

    /// The meeting was discarded: nothing more is written.
    pub fn discard(self) {
        self.discarded.store(true, Ordering::SeqCst);
    }
}

pub(crate) fn meeting(capacity: usize) -> (LiveMeeting, mpsc::Receiver<Chunk>) {
    let (sender, receiver) = mpsc::channel(capacity);
    let meeting = LiveMeeting {
        sender,
        headphones: Arc::default(),
        discarded: Arc::default(),
    };
    (meeting, receiver)
}

/// Starts the live transcript of meeting `id`, recorded in `dir` from
/// `sources`. `None` when there is nothing to transcribe live: the state
/// then says why.
pub fn start(
    live: &Arc<Live>,
    engine: Option<Arc<dyn LiveEngine>>,
    id: i64,
    dir: PathBuf,
    sources: &[Source],
    headphones: bool,
) -> Option<LiveMeeting> {
    let Some(engine) = engine else {
        live.begin(
            id,
            LiveState::Unavailable {
                reason: LiveUnavailable::Off,
            },
        );
        return None;
    };
    let state = match engine.availability() {
        Ok(()) => {
            engine.warm();
            LiveState::Starting
        }
        Err(reason) => LiveState::Unavailable { reason },
    };
    live.begin(id, state);
    if matches!(
        state,
        LiveState::Unavailable {
            reason: LiveUnavailable::Unsupported
        }
    ) {
        return None;
    }
    let (meeting, receiver) = meeting(QUEUE_CHUNKS);
    meeting.set_headphones(headphones);
    let run = worker::Worker::new(
        worker::Out {
            id,
            dir,
            live: Arc::clone(live),
            discarded: Arc::clone(&meeting.discarded),
        },
        engine,
        sources.contains(&Source::System),
        Arc::clone(&meeting.headphones),
    );
    tauri::async_runtime::spawn(run.run(receiver));
    Some(meeting)
}

/// The dictation's local engine, shared: one helper and one model copy.
pub struct AppEngine(pub tauri::AppHandle);

impl AppEngine {
    fn engine(&self) -> Option<Arc<crate::engine::local::LocalEngine>> {
        use tauri::Manager;
        self.0
            .try_state::<Arc<crate::engine::local::LocalEngine>>()
            .map(|engine| Arc::clone(&engine))
    }
}

impl LiveEngine for AppEngine {
    fn availability(&self) -> Result<(), LiveUnavailable> {
        use crate::local_models::{LocalModelState, LocalModels};
        use tauri::Manager;
        if !cfg!(target_arch = "aarch64") {
            return Err(LiveUnavailable::Unsupported);
        }
        let Some(models) = self.0.try_state::<Arc<LocalModels>>() else {
            return Err(LiveUnavailable::Unsupported);
        };
        match models.state() {
            LocalModelState::Ready if self.engine().is_some() => Ok(()),
            LocalModelState::Ready | LocalModelState::Unsupported { .. } => {
                Err(LiveUnavailable::Unsupported)
            }
            LocalModelState::Missing | LocalModelState::Failed { .. } => {
                Err(LiveUnavailable::Missing)
            }
            LocalModelState::Downloading { .. } => Err(LiveUnavailable::Downloading),
            LocalModelState::Preparing | LocalModelState::Optimizing => {
                Err(LiveUnavailable::Preparing)
            }
        }
    }

    fn warm(&self) {
        if let Some(engine) = self.engine() {
            engine.warm();
        }
    }

    fn transcribe<'a>(
        &'a self,
        samples: &'a [f32],
    ) -> crate::engine::local::session::BoxFuture<'a, crate::engine::local::session::Background>
    {
        Box::pin(async move {
            match self.engine() {
                Some(engine) => engine.transcribe_live(samples).await,
                None => crate::engine::local::session::Background::Failed(anyhow::anyhow!(
                    "Local transcription is unavailable in this build"
                )),
            }
        })
    }

    fn keep_loaded(&self) {
        if let Some(engine) = self.engine() {
            engine.keep_loaded();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: Source, text: &str, start_ms: u64) -> LiveEntry {
        LiveEntry {
            source,
            start_ms,
            end_ms: start_ms + 1_000,
            text: text.into(),
            kind: LiveEntryKind::Speech,
        }
    }

    #[test]
    fn a_crashed_meeting_shows_its_draft_until_the_final_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let live = Live::new(Box::new(|_| {}));
        assert_eq!(transcript_of(&live, dir.path(), 7), None);

        draft::append(
            dir.path(),
            &[
                entry(Source::Mic, "Bonjour à tous.", 1_000),
                entry(Source::System, "Salut.", 2_500),
            ],
        )
        .unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join(draft::DRAFT_FILE))
            .unwrap();
        std::io::Write::write_all(&mut file, br#"{"source":"mic","start_ms":40"#).unwrap();

        let recovered = transcript_of(&live, dir.path(), 7).unwrap();
        assert_eq!(recovered.state, LiveState::Stopped);
        assert_eq!(
            recovered
                .entries
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            ["Bonjour à tous.", "Salut."]
        );

        draft::remove(dir.path()).unwrap();
        assert_eq!(transcript_of(&live, dir.path(), 7), None);
        live.begin(7, LiveState::Stopped);
        assert_eq!(transcript_of(&live, dir.path(), 7), None);
    }

    #[test]
    fn paste_takes_what_was_said_since_the_last_paste() {
        let live = Live::new(Box::new(|_| {}));
        live.begin(3, LiveState::Starting);
        assert!(live.take_new().is_empty());
        live.update(
            3,
            LiveState::Live,
            vec![
                entry(Source::System, "Others first.", 2_000),
                entry(Source::Mic, "Me before.", 1_000),
            ],
        );
        let first = live.take_new();
        assert_eq!(
            paste_text(&first, "You", "Others"),
            "You: Me before.\nOthers: Others first."
        );
        assert!(live.take_new().is_empty(), "nothing new yet");
        live.update(
            3,
            LiveState::Live,
            vec![entry(Source::Mic, "Then this.", 5_000)],
        );
        assert_eq!(
            paste_text(&live.take_new(), "You", "Others"),
            "You: Then this."
        );
    }
}
