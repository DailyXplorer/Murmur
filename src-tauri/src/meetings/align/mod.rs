//! Places Codex text on the timeline using the times of the words the
//! on-device recogniser heard. That recogniser's text is never shown.

pub mod dp;
pub mod timing;

use super::tokens::Token;

/// A word the on-device recogniser heard, by its comparison key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedWord {
    pub key: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A Codex word with its time on the track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedToken {
    pub token: Token,
    pub start_ms: u64,
    pub end_ms: u64,
    /// The time is estimated rather than measured.
    pub low_confidence: bool,
}
