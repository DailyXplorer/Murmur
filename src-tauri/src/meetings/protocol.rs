//! What `murmur-meetings-helper` prints: one JSON object per line on stdout.
//! Unknown message types are ignored, so the helper can grow without
//! breaking an older app.

use serde::{Deserialize, Serialize};

use super::align::TimedWord;
use super::merge::Track;
use super::speakers::Segment;
use super::tokens::key;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Download,
    /// The first load of the models, compiled for this Mac.
    Compile,
    Asr,
    Diarize,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ModelMissing,
    Unsupported,
    Io,
    Crash,
    #[serde(other)]
    Other,
}

/// A word heard on device, with its times in seconds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Word {
    pub w: String,
    pub s: f64,
    pub e: f64,
}

/// One diarized turn, with its times in seconds.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Turn {
    pub spk: String,
    pub s: f64,
    pub e: f64,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Progress {
        phase: Phase,
        #[serde(default)]
        track: Option<Track>,
        fraction: f64,
    },
    Models {
        ready: bool,
        bytes: u64,
    },
    Words {
        track: Track,
        words: Vec<Word>,
    },
    Diarization {
        track: Track,
        segments: Vec<Turn>,
    },
    Error {
        stage: String,
        code: ErrorCode,
        #[serde(default)]
        message: String,
    },
    Done,
    #[serde(other)]
    Unknown,
}

pub fn parse_line(line: &str) -> serde_json::Result<Message> {
    serde_json::from_str(line)
}

/// What Murmur asks `serve`, one JSON object per line on its stdin.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub enum Request<'a> {
    /// Transcribe a 16 kHz mono WAV of at most 15 s, listening for the
    /// custom words in `terms` and, held to a closer match, the window
    /// `names`. A helper that knows no `names` reads the request without.
    Transcribe {
        id: u64,
        wav: &'a std::path::Path,
        #[serde(skip_serializing_if = "<[String]>::is_empty")]
        terms: &'a [String],
        #[serde(skip_serializing_if = "<[String]>::is_empty")]
        names: &'a [String],
    },
}

/// What `serve` prints, one JSON object per line on its stdout.
#[derive(Deserialize, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub enum Reply {
    /// The models are loaded: requests are answered from now on.
    Ready,
    Text {
        id: u64,
        text: String,
    },
    /// A failed request, or without `id` a failed load.
    Error {
        #[serde(default)]
        id: Option<u64>,
        code: ErrorCode,
        #[serde(default)]
        message: String,
    },
    #[serde(other)]
    Unknown,
}

#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub fn parse_reply(line: &str) -> serde_json::Result<Reply> {
    serde_json::from_str(line)
}

fn ms(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

/// The words in time order, by comparison key. Punctuation is dropped.
pub fn timed_words(words: &[Word]) -> Vec<TimedWord> {
    let mut timed: Vec<TimedWord> = words
        .iter()
        .map(|word| TimedWord {
            key: key(&word.w),
            start_ms: ms(word.s),
            end_ms: ms(word.e.max(word.s)),
        })
        .filter(|word| !word.key.is_empty())
        .collect();
    timed.sort_by_key(|word| word.start_ms);
    timed
}

pub fn segments(turns: &[Turn]) -> Vec<Segment> {
    turns
        .iter()
        .filter(|turn| turn.e > turn.s)
        .map(|turn| Segment {
            speaker: turn.spk.clone(),
            start_ms: ms(turn.s),
            end_ms: ms(turn.e),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_type_parses() {
        assert_eq!(
            parse_line(r#"{"type":"progress","phase":"download","fraction":0.42}"#).unwrap(),
            Message::Progress {
                phase: Phase::Download,
                track: None,
                fraction: 0.42
            }
        );
        assert_eq!(
            parse_line(r#"{"type":"progress","phase":"asr","track":"system","fraction":1}"#)
                .unwrap(),
            Message::Progress {
                phase: Phase::Asr,
                track: Some(Track::System),
                fraction: 1.0
            }
        );
        assert_eq!(
            parse_line(r#"{"type":"models","ready":true,"missing":[],"bytes":512}"#).unwrap(),
            Message::Models {
                ready: true,
                bytes: 512
            }
        );
        assert_eq!(
            parse_line(r#"{"type":"words","track":"mic","words":[{"w":"Plateforme,","s":12.32,"e":12.81,"c":0.9}]}"#)
                .unwrap(),
            Message::Words {
                track: Track::Mic,
                words: vec![Word {
                    w: "Plateforme,".into(),
                    s: 12.32,
                    e: 12.81
                }]
            }
        );
        assert_eq!(
            parse_line(r#"{"type":"diarization","track":"system","segments":[{"spk":"S1","s":10.1,"e":14.9,"q":0.8}]}"#)
                .unwrap(),
            Message::Diarization {
                track: Track::System,
                segments: vec![Turn {
                    spk: "S1".into(),
                    s: 10.1,
                    e: 14.9
                }]
            }
        );
        assert_eq!(
            parse_line(r#"{"type":"error","stage":"diarize","code":"crash","message":"boom"}"#)
                .unwrap(),
            Message::Error {
                stage: "diarize".into(),
                code: ErrorCode::Crash,
                message: "boom".into()
            }
        );
        assert_eq!(parse_line(r#"{"type":"done"}"#).unwrap(), Message::Done);
    }

    #[test]
    fn unknown_types_and_values_are_tolerated() {
        assert_eq!(
            parse_line(r#"{"type":"vad","track":"mic","segments":[[0.5,3.2]]}"#).unwrap(),
            Message::Unknown
        );
        assert_eq!(
            parse_line(r#"{"type":"progress","phase":"warmup","fraction":0.1}"#).unwrap(),
            Message::Progress {
                phase: Phase::Other,
                track: None,
                fraction: 0.1
            }
        );
        assert_eq!(
            parse_line(r#"{"type":"error","stage":"asr","code":"new_code"}"#).unwrap(),
            Message::Error {
                stage: "asr".into(),
                code: ErrorCode::Other,
                message: String::new()
            }
        );
    }

    #[test]
    fn serve_requests_and_replies_round_trip() {
        assert_eq!(
            serde_json::to_string(&Request::Transcribe {
                id: 7,
                wav: std::path::Path::new("/tmp/piece 7.wav"),
                terms: &[],
                names: &[],
            })
            .unwrap(),
            r#"{"type":"transcribe","id":7,"wav":"/tmp/piece 7.wav"}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Transcribe {
                id: 8,
                wav: std::path::Path::new("/tmp/piece 8.wav"),
                terms: &["Siobhán".to_string(), "Kubernetes".to_string()],
                names: &[],
            })
            .unwrap(),
            r#"{"type":"transcribe","id":8,"wav":"/tmp/piece 8.wav","terms":["Siobhán","Kubernetes"]}"#
        );
        assert_eq!(
            serde_json::to_string(&Request::Transcribe {
                id: 9,
                wav: std::path::Path::new("/tmp/piece 9.wav"),
                terms: &["Siobhán".to_string()],
                names: &["Ingrid".to_string()],
            })
            .unwrap(),
            r#"{"type":"transcribe","id":9,"wav":"/tmp/piece 9.wav","terms":["Siobhán"],"names":["Ingrid"]}"#
        );
        assert_eq!(parse_reply(r#"{"type":"ready"}"#).unwrap(), Reply::Ready);
        assert_eq!(
            parse_reply(r#"{"type":"text","id":7,"text":"Bonjour à tous."}"#).unwrap(),
            Reply::Text {
                id: 7,
                text: "Bonjour à tous.".into()
            }
        );
        assert_eq!(
            parse_reply(
                r#"{"type":"error","id":7,"stage":"transcribe","code":"io","message":"bad wav"}"#
            )
            .unwrap(),
            Reply::Error {
                id: Some(7),
                code: ErrorCode::Io,
                message: "bad wav".into()
            }
        );
        assert_eq!(
            parse_reply(r#"{"type":"error","stage":"load","code":"model_missing"}"#).unwrap(),
            Reply::Error {
                id: None,
                code: ErrorCode::ModelMissing,
                message: String::new()
            }
        );
        assert_eq!(
            parse_reply(r#"{"type":"progress","phase":"compile","fraction":0.5}"#).unwrap(),
            Reply::Unknown
        );
        for line in ["not json", r#"{"type":"text","id":7}"#, r#"{"id":7}"#] {
            assert!(parse_reply(line).is_err(), "{line}");
        }
    }

    #[test]
    fn malformed_lines_are_errors_not_panics() {
        for line in [
            "",
            "not json",
            "{",
            r#"{"type":"words","track":"mic"}"#,
            "[1,2]",
        ] {
            assert!(parse_line(line).is_err(), "{line}");
        }
    }

    #[test]
    fn words_become_sorted_millisecond_keys_without_punctuation() {
        let words = [
            Word {
                w: "Jeudi.".into(),
                s: 2.0,
                e: 2.4,
            },
            Word {
                w: "–".into(),
                s: 1.5,
                e: 1.5,
            },
            Word {
                w: "Livré".into(),
                s: 1.0,
                e: 1.4004,
            },
        ];
        assert_eq!(
            timed_words(&words),
            [
                TimedWord {
                    key: "livre".into(),
                    start_ms: 1_000,
                    end_ms: 1_400
                },
                TimedWord {
                    key: "jeudi".into(),
                    start_ms: 2_000,
                    end_ms: 2_400
                },
            ]
        );
        let turns = [
            Turn {
                spk: "S1".into(),
                s: 0.0,
                e: 1.25,
            },
            Turn {
                spk: "S2".into(),
                s: 3.0,
                e: 3.0,
            },
        ];
        assert_eq!(
            segments(&turns),
            [Segment {
                speaker: "S1".into(),
                start_ms: 0,
                end_ms: 1_250
            }]
        );
    }
}
