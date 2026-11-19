//! The messages of ChatGPT's dictation stream, each a JSON text frame. Only
//! the fields Murmur sends or reads are modelled.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(super) enum ClientMessage<'a> {
    #[serde(rename = "session.start")]
    Start {
        dictation_session_id: &'a str,
        attempt_id: &'a str,
        config: Config<'a>,
    },
    /// Base64 of little-endian PCM16 mono.
    #[serde(rename = "audio.append")]
    Audio { audio: String },
    #[serde(rename = "session.close")]
    Close,
}

#[derive(Debug, Serialize)]
pub(super) struct Config<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
    input_audio_format: &'static str,
    sample_rate_hz: u32,
    num_channels: u8,
    max_buffer_size_bytes: u32,
    max_utterance_duration_ms: u32,
    session_ttl_ms: u32,
    provider_mode: &'static str,
    transcript_delivery_mode: Delivery,
    vad: Vad,
}

/// When the server sends the text of an utterance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Delivery {
    /// Once, when the utterance is over.
    FinalOnly,
    /// Each time more of it is heard, whole so far, then the final text.
    Segment,
}

#[derive(Debug, Serialize)]
struct Vad {
    #[serde(rename = "type")]
    kind: &'static str,
    threshold: f32,
    prefix_padding_ms: u32,
    silence_duration_ms: u32,
}

impl<'a> Config<'a> {
    /// What Murmur asks of every session. Without a language the server
    /// detects it. The buffer and utterance sizes are the most the server
    /// grants, and neither cuts a long dictation. The session may live 30
    /// minutes, twice the longest recording.
    pub(super) fn dictation(language: Option<&'a str>, delivery: Delivery) -> Self {
        Self {
            language,
            input_audio_format: "pcm16",
            sample_rate_hz: 16_000,
            num_channels: 1,
            max_buffer_size_bytes: 4_194_304,
            max_utterance_duration_ms: 30_000,
            session_ttl_ms: 1_800_000,
            provider_mode: "streaming_sse",
            transcript_delivery_mode: delivery,
            vad: Vad {
                kind: "server_vad",
                threshold: 0.5,
                prefix_padding_ms: 300,
                silence_duration_ms: 500,
            },
        }
    }
}

/// An event of a type Murmur does not act on is `Other`.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub(super) enum ServerEvent {
    #[serde(rename = "session.started")]
    Started,
    #[serde(rename = "session.updated")]
    Updated { session: SessionInfo },
    #[serde(rename = "speech.started")]
    SpeechStarted { utterance_id: String },
    #[serde(rename = "transcript.segment")]
    Segment(UtteranceText),
    #[serde(rename = "transcript.final")]
    Final(UtteranceText),
    #[serde(rename = "transcript.failed")]
    TranscriptFailed,
    #[serde(rename = "session.error")]
    Error {
        #[serde(default)]
        fatal: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(super) struct SessionInfo {
    status: String,
}

impl SessionInfo {
    /// Sent once, after `session.close`: the session's last word.
    pub(super) fn is_closed(&self) -> bool {
        self.status == "closed"
    }
}

/// The whole text of an utterance as of `revision`.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(super) struct UtteranceText {
    pub(super) utterance_id: String,
    pub(super) revision: u64,
    pub(super) text: String,
}

pub(super) fn parse(frame: &str) -> serde_json::Result<ServerEvent> {
    serde_json::from_str(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(language: Option<&str>, delivery: Delivery) -> String {
        serde_json::to_string(&ClientMessage::Start {
            dictation_session_id: "11111111-1111-4111-8111-111111111111",
            attempt_id: "22222222-2222-4222-8222-222222222222",
            config: Config::dictation(language, delivery),
        })
        .unwrap()
    }

    #[test]
    fn session_start_has_the_shape_the_server_was_tested_with() {
        assert_eq!(
            start(Some("fr"), Delivery::FinalOnly),
            concat!(
                r#"{"type":"session.start","#,
                r#""dictation_session_id":"11111111-1111-4111-8111-111111111111","#,
                r#""attempt_id":"22222222-2222-4222-8222-222222222222","#,
                r#""config":{"language":"fr","input_audio_format":"pcm16","#,
                r#""sample_rate_hz":16000,"num_channels":1,"#,
                r#""max_buffer_size_bytes":4194304,"max_utterance_duration_ms":30000,"#,
                r#""session_ttl_ms":1800000,"provider_mode":"streaming_sse","#,
                r#""transcript_delivery_mode":"final_only","#,
                r#""vad":{"type":"server_vad","threshold":0.5,"#,
                r#""prefix_padding_ms":300,"silence_duration_ms":500}}}"#
            )
        );
    }

    #[test]
    fn session_start_asks_for_the_text_as_it_is_heard_only_for_a_listener() {
        let mode = |delivery| {
            let message: serde_json::Value = serde_json::from_str(&start(None, delivery)).unwrap();
            message["config"]["transcript_delivery_mode"].clone()
        };
        assert_eq!(mode(Delivery::Segment), "segment");
        assert_eq!(mode(Delivery::FinalOnly), "final_only");
    }

    #[test]
    fn session_start_leaves_the_language_out_for_automatic_detection() {
        let message = start(None, Delivery::FinalOnly);
        assert!(!message.contains("language"), "{message}");
        assert!(message.contains(r#""config":{"input_audio_format":"pcm16""#));
    }

    #[test]
    fn audio_and_close_are_the_two_other_client_messages() {
        let audio = ClientMessage::Audio {
            audio: "AAA=".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&audio).unwrap(),
            r#"{"type":"audio.append","audio":"AAA="}"#
        );
        assert_eq!(
            serde_json::to_string(&ClientMessage::Close).unwrap(),
            r#"{"type":"session.close"}"#
        );
    }

    fn text(utterance_id: &str, revision: u64, text: &str) -> UtteranceText {
        UtteranceText {
            utterance_id: utterance_id.to_string(),
            revision,
            text: text.to_string(),
        }
    }

    #[test]
    fn each_event_murmur_acts_on_parses() {
        let closed = |status: &str| {
            let frame = format!(
                r#"{{"type":"session.updated","sequence_no":9,"session":{{"session_id":"s","status":"{status}","config":{{}}}}}}"#
            );
            match parse(&frame).unwrap() {
                ServerEvent::Updated { session } => session.is_closed(),
                other => panic!("{other:?}"),
            }
        };
        assert!(!closed("active"));
        assert!(closed("closed"));

        let rows = [
            (
                r#"{"type":"session.started","sequence_no":1,"session":{"session_id":"s","status":"active","config":{}},"frame_rtt_supported":true}"#,
                ServerEvent::Started,
            ),
            (
                r#"{"type":"speech.started","sequence_no":2,"utterance_id":"utt_1"}"#,
                ServerEvent::SpeechStarted {
                    utterance_id: "utt_1".to_string(),
                },
            ),
            (
                r#"{"type":"transcript.segment","sequence_no":3,"utterance_id":"utt_1","revision":2,"text":" Bonjour"}"#,
                ServerEvent::Segment(text("utt_1", 2, " Bonjour")),
            ),
            (
                r#"{"type":"transcript.final","sequence_no":4,"utterance_id":"utt_1","revision":3,"text":"Bonjour."}"#,
                ServerEvent::Final(text("utt_1", 3, "Bonjour.")),
            ),
            (
                r#"{"type":"transcript.failed","sequence_no":5,"utterance_id":null,"error":{"code":"c","message":"m","retryable":false}}"#,
                ServerEvent::TranscriptFailed,
            ),
            (
                r#"{"type":"session.error","sequence_no":6,"fatal":true,"error":{"code":"c","message":"m","retryable":false}}"#,
                ServerEvent::Error { fatal: true },
            ),
            (
                r#"{"type":"session.error","fatal":false,"error":{"code":"invalid_event","message":"m","retryable":false}}"#,
                ServerEvent::Error { fatal: false },
            ),
        ];
        for (frame, event) in rows {
            assert_eq!(parse(frame).unwrap(), event, "{frame}");
        }
    }

    #[test]
    fn an_event_of_another_type_is_other() {
        for kind in [
            "speech.stopped",
            "transcript.delta",
            "session.stop_requested",
            "asset.ready",
            "asset.committed",
            "asset.failed",
            "audio.frame_processed",
            "something.new",
        ] {
            let frame = format!(r#"{{"type":"{kind}","sequence_no":7,"utterance_id":"utt_1"}}"#);
            assert_eq!(parse(&frame).unwrap(), ServerEvent::Other, "{kind}");
        }
    }

    #[test]
    fn a_frame_that_is_not_an_event_does_not_parse() {
        for frame in [
            "",
            "not json",
            "[]",
            r#"{"sequence_no":1}"#,
            r#"{"type":"transcript.final","utterance_id":"utt_1"}"#,
            r#"{"type":"session.updated"}"#,
        ] {
            assert!(parse(frame).is_err(), "{frame}");
        }
    }
}
