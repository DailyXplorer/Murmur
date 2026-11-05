use serde::{Deserialize, Serialize};
use specta::Type;

use crate::meetings::merge::Track;
use crate::meetings::summary::SummaryError;
use crate::search::Snippet;

/// Which tracks a meeting recorded.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingTracks {
    Both,
    System,
    Mic,
}

impl MeetingTracks {
    pub fn from_parts(mic: bool, system: bool) -> Option<Self> {
        match (mic, system) {
            (true, true) => Some(Self::Both),
            (true, false) => Some(Self::Mic),
            (false, true) => Some(Self::System),
            (false, false) => None,
        }
    }

    pub fn has_mic(self) -> bool {
        self != Self::System
    }

    pub fn has_system(self) -> bool {
        self != Self::Mic
    }

    pub(super) fn as_db(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::System => "system",
            Self::Mic => "mic",
        }
    }

    pub(super) fn from_db(value: &str) -> Option<Self> {
        match value {
            "both" => Some(Self::Both),
            "system" => Some(Self::System),
            "mic" => Some(Self::Mic),
            _ => None,
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Recording,
    /// The app quit or crashed while recording; the tracks were repaired.
    Interrupted,
    /// Recorded and waiting for (or in) processing.
    Processing,
    Ready,
    Failed,
}

impl MeetingStatus {
    pub(super) fn from_db(value: &str) -> Option<Self> {
        match value {
            "recording" => Some(Self::Recording),
            "interrupted" => Some(Self::Interrupted),
            "processing" => Some(Self::Processing),
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl MeetingStatus {
    pub(super) fn as_db(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Interrupted => "interrupted",
            Self::Processing => "processing",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

/// What processing is doing now.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStage {
    Transcribing,
    /// Telling speakers apart on device, once Codex is done.
    Analyzing,
    /// Placing the text of both tracks in order.
    Aligning,
}

impl MeetingStage {
    pub(super) fn as_db(self) -> &'static str {
        match self {
            Self::Transcribing => "transcribing",
            Self::Analyzing => "analyzing",
            Self::Aligning => "aligning",
        }
    }

    pub(super) fn from_db(value: &str) -> Option<Self> {
        match value {
            "transcribing" => Some(Self::Transcribing),
            "analyzing" => Some(Self::Analyzing),
            "aligning" => Some(Self::Aligning),
            _ => None,
        }
    }
}

/// How the speakers of a meeting were told apart.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerAnalysis {
    /// On-device diarization and word times were used.
    Separated,
    /// The models were not downloaded: one speaker per track.
    NoModels,
    /// The analysis failed or crashed: one speaker per track.
    Failed,
    /// This Mac cannot run the analysis: one speaker per track.
    Unsupported,
}

impl SpeakerAnalysis {
    pub(super) fn as_db(self) -> &'static str {
        match self {
            Self::Separated => "separated",
            Self::NoModels => "no_models",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        }
    }

    pub(super) fn from_db(value: &str) -> Option<Self> {
        match value {
            "separated" => Some(Self::Separated),
            "no_models" => Some(Self::NoModels),
            "failed" => Some(Self::Failed),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// Why the last processing run did not finish everything.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingError {
    /// Codex needs a new sign-in.
    SignedOut,
    /// Codex kept refusing requests for too long.
    RateLimited,
    /// Some parts could not be transcribed; the rest is shown.
    PartsFailed,
    /// No track could be read.
    NoAudio,
    /// Codex could not be reached: processing goes on by itself once the
    /// connection is back.
    Offline,
    /// Processing stopped on an unexpected error.
    Other,
}

impl ProcessingError {
    pub(super) fn as_db(self) -> &'static str {
        match self {
            Self::SignedOut => "signed_out",
            Self::RateLimited => "rate_limited",
            Self::PartsFailed => "parts_failed",
            Self::NoAudio => "no_audio",
            Self::Offline => "offline",
            Self::Other => "other",
        }
    }

    pub(super) fn from_db(value: &str) -> Option<Self> {
        match value {
            "signed_out" => Some(Self::SignedOut),
            "rate_limited" => Some(Self::RateLimited),
            "parts_failed" => Some(Self::PartsFailed),
            "no_audio" => Some(Self::NoAudio),
            "offline" => Some(Self::Offline),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SummaryStatus {
    /// Not written yet: it is once the whole transcript is.
    Pending,
    Ready,
    Failed,
}

impl SummaryStatus {
    pub(super) fn from_db(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

/// Where a meeting's audio is.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioState {
    /// The tracks as recorded, until processing compresses them.
    Tracks,
    /// `audio.m4a`.
    Compressed,
    /// The user deleted it.
    Deleted,
}

pub(super) fn track_db(track: Track) -> &'static str {
    match track {
        Track::Mic => "mic",
        Track::System => "system",
    }
}

pub(super) fn track_from_db(value: &str) -> Option<Track> {
    match value {
        "mic" => Some(Track::Mic),
        "system" => Some(Track::System),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkStatus {
    Pending,
    Done,
    Failed,
    /// No speech: never uploaded.
    Silent,
}

impl ChunkStatus {
    pub(super) fn from_db(value: &str) -> Self {
        match value {
            "done" => Self::Done,
            "failed" => Self::Failed,
            "silent" => Self::Silent,
            _ => Self::Pending,
        }
    }
}

/// One piece of a track and what Codex made of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRow {
    pub track: Track,
    pub idx: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub status: ChunkStatus,
    pub text: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct MeetingSpeaker {
    /// `me`, `s1`, `s2`…: stable, so a rename touches no text.
    pub key: String,
    pub name: String,
    pub default_name: String,
    pub color_index: u32,
}

/// One bubble of the transcript. A `_system` speaker marks a part that
/// could not be transcribed.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct MeetingTurn {
    pub speaker_key: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub low_confidence: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingDetail {
    pub id: i64,
    pub title: Option<String>,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    pub status: MeetingStatus,
    pub tracks: MeetingTracks,
    pub stage: Option<MeetingStage>,
    pub error: Option<ProcessingError>,
    /// `None` until the transcript is first built.
    pub speaker_analysis: Option<SpeakerAnalysis>,
    pub speakers: Vec<MeetingSpeaker>,
    pub turns: Vec<MeetingTurn>,
    /// Markdown with `[[key]]` markers where people are named.
    pub summary: Option<String>,
    pub summary_status: SummaryStatus,
    pub summary_error: Option<SummaryError>,
    pub audio: AudioState,
    /// The size of `audio.m4a`, once compressed.
    pub audio_bytes: i64,
    /// System audio was recorded as pure silence while the user spoke, which
    /// is what macOS gives when Murmur may not record system audio.
    pub system_audio_silent: bool,
    /// When a Codex rate-limit hold stops keeping this queued meeting
    /// waiting. Only the processing queue knows; the store never sets it.
    pub held_until_ms: Option<i64>,
    /// This queued meeting waits for the connection to reach Codex. Only the
    /// processing queue knows; the store never sets it.
    pub waiting_for_connection: bool,
    /// How much of the recorded audio this Mac transcribed because Codex
    /// refused it. "Transcribe again" sends it all to Codex once more.
    pub local_fallback_ms: i64,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingListItem {
    pub id: i64,
    /// `None` until a summary names the meeting.
    pub title: Option<String>,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    pub status: MeetingStatus,
    pub tracks: MeetingTracks,
}

#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingsPage {
    pub meetings: Vec<MeetingListItem>,
    pub has_more: bool,
}

/// A meeting that matched a search.
#[derive(Serialize, Debug, Clone, PartialEq, Type)]
pub struct MeetingHit {
    pub id: i64,
    /// `None` until a summary names the meeting.
    pub title: Option<Snippet>,
    pub started_at_ms: i64,
    pub duration_ms: i64,
    pub status: MeetingStatus,
    /// Up to two passages: the summary's, then the first matching turns.
    pub snippets: Vec<Snippet>,
    /// The index in `MeetingDetail::turns` of the first matching turn (of
    /// the newest it read, for a query matching very many turns).
    pub first_turn_seq: Option<i64>,
}
