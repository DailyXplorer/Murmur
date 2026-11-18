//! The task that owns one dictation's socket: it opens the session, sends
//! the audio the recorder hands over, and reads events until the session
//! closes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{SinkExt, StreamExt};
use log::debug;
use reqwest_websocket::{Message, Upgrade, WebSocket};
use tokio::sync::watch;
use tokio::time::Instant;

use super::decision::{Failure, Standing};
use super::feed::Audio;
use super::protocol::{self, ClientMessage, Config, ServerEvent};
use super::transcript::Transcript;
use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::codex_auth::{self, Session};
use crate::codex_transcribe::{to_pcm16, user_agent, CONNECT_TIMEOUT};

/// The only subprotocol sent. With `codex-desktop` beside it, as the
/// official app sends, the server answered 403.
const SUBPROTOCOL: &str = "chatgpt-dictation";

/// Four recorder frames of 30 ms, close to the 100 ms messages the server
/// was measured with.
const MESSAGE_SAMPLES: usize = 1_920;

/// The official client ends the audio with 250 ms of silence, which lets the
/// server's voice detection close the last utterance.
const CLOSING_SILENCE: usize = TARGET_SAMPLE_RATE as usize / 4;

pub(super) struct Params {
    pub(super) url: String,
    pub(super) credentials: Credentials,
    /// As a request names it; `None` lets the server detect the language.
    pub(super) language: Option<String>,
}

pub(super) enum Credentials {
    /// The session in this auth.json (the Codex default when `None`),
    /// refreshed first when it is about to expire.
    AuthFile(Option<PathBuf>),
    /// A live test never refreshes the tokens every Codex client shares.
    #[cfg(test)]
    Session(Session),
}

/// Where a session is. The task publishes each step; `Done` is the last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum State {
    /// From the task's start until `session.started`.
    Connecting,
    /// Audio goes out as the recorder hands it over.
    Streaming {
        since: Instant,
    },
    /// `session.close` was sent.
    Closing {
        since: Instant,
    },
    Done(End),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum End {
    /// The session closed with every utterance final: its text, maybe empty.
    Closed(String),
    /// More audio waited than a stream may be behind.
    Lagging,
    Failed(Failure),
}

impl From<Failure> for End {
    fn from(failure: Failure) -> Self {
        Self::Failed(failure)
    }
}

impl From<End> for Standing {
    fn from(end: End) -> Self {
        match end {
            End::Closed(text) => Self::Closed(text),
            End::Lagging => Self::Lagging,
            End::Failed(failure) => Self::Failed(failure),
        }
    }
}

/// Runs one session to its end. `aborted` is set before the task is
/// aborted, so an end of audio that follows an abort sends nothing more.
pub(super) async fn run(
    params: Params,
    mut audio: Audio,
    aborted: Arc<AtomicBool>,
    state: watch::Sender<State>,
) {
    let began = Instant::now();
    let end = match stream(&params, &mut audio, &aborted, &state, began).await {
        Ok(text) => End::Closed(text),
        Err(end) => end,
    };
    debug!(
        "Cloud stream session ended after {} ms",
        began.elapsed().as_millis()
    );
    state.send_replace(State::Done(end));
}

async fn stream(
    params: &Params,
    audio: &mut Audio,
    aborted: &AtomicBool,
    state: &watch::Sender<State>,
    began: Instant,
) -> Result<String, End> {
    let mut socket = open(params).await?;
    let mut transcript = Transcript::default();
    // Samples taken from the recorder that do not fill a message yet.
    let mut partial_message: Vec<f32> = Vec::with_capacity(MESSAGE_SAMPLES);

    loop {
        let streaming = matches!(*state.borrow(), State::Streaming { .. });
        tokio::select! {
            frame = socket.next() => match event_of(frame)? {
                ServerEvent::Started => {
                    if matches!(*state.borrow(), State::Connecting) {
                        debug!(
                            "Cloud stream session started in {} ms",
                            began.elapsed().as_millis()
                        );
                        state.send_replace(State::Streaming { since: Instant::now() });
                    }
                }
                ServerEvent::Updated { session } if session.is_closed() => {
                    if !matches!(*state.borrow(), State::Closing { .. }) {
                        return Err(Failure::ClosedEarly.into());
                    }
                    return transcript
                        .final_text()
                        .ok_or(End::Failed(Failure::Unfinished));
                }
                ServerEvent::SpeechStarted { utterance_id } => transcript.heard(&utterance_id),
                ServerEvent::Segment(text) => {
                    transcript.segment(&text.utterance_id, text.revision, &text.text);
                }
                ServerEvent::Final(text) => {
                    transcript.finish(&text.utterance_id, text.revision, &text.text);
                }
                ServerEvent::TranscriptFailed => return Err(Failure::TranscriptFailed.into()),
                ServerEvent::Error { fatal } => {
                    // Before the session started, the error answers
                    // `session.start`: no `session.started` follows it.
                    if fatal || matches!(*state.borrow(), State::Connecting) {
                        return Err(Failure::SessionError.into());
                    }
                }
                ServerEvent::Updated { .. } | ServerEvent::Other => {}
            },
            // Until the session started, the frames wait in the channel.
            frame = audio.frames.recv(), if streaming => {
                if audio.meter.overflowed() {
                    return Err(End::Lagging);
                }
                match frame {
                    Some(frame) => {
                        partial_message.extend_from_slice(&frame);
                        while partial_message.len() >= MESSAGE_SAMPLES {
                            send_audio(&mut socket, audio, &partial_message[..MESSAGE_SAMPLES])
                                .await?;
                            partial_message.drain(..MESSAGE_SAMPLES);
                        }
                    }
                    None => {
                        if aborted.load(Ordering::Acquire) {
                            return Err(Failure::ClosedEarly.into());
                        }
                        if !partial_message.is_empty() {
                            send_audio(&mut socket, audio, &partial_message).await?;
                            partial_message.clear();
                        }
                        send_audio(&mut socket, audio, &vec![0.0; CLOSING_SILENCE]).await?;
                        send(&mut socket, &ClientMessage::Close).await?;
                        state.send_replace(State::Closing { since: Instant::now() });
                    }
                }
            }
        }
    }
}

/// Connects, upgrades and sends `session.start`. The handshake names no
/// client: the Codex identity the upload sends could not be verified here,
/// so the stream counts against ChatGPT's own dictation quota.
pub(super) async fn open(params: &Params) -> Result<WebSocket, Failure> {
    let session = session(&params.credentials).await?;
    // Its own client: an upgrade needs HTTP/1.1, and the shared client
    // speaks HTTP/2 to the same host.
    let client = reqwest::Client::builder()
        .http1_only()
        .user_agent(user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|_| Failure::Connect)?;
    let mut request = client
        .get(&params.url)
        .header("Authorization", format!("Bearer {}", session.access_token));
    if let Some(account_id) = session.account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    let response = request
        .upgrade()
        .protocols([SUBPROTOCOL])
        .send()
        .await
        .map_err(|_| Failure::Connect)?;
    let status = response.status();
    if status != reqwest::StatusCode::SWITCHING_PROTOCOLS {
        return Err(Failure::Refused(status.as_u16()));
    }
    let mut socket = response
        .into_websocket()
        .await
        .map_err(|_| Failure::Connect)?;

    let (session_id, attempt_id) = (uuid_v4(), uuid_v4());
    let start = ClientMessage::Start {
        dictation_session_id: &session_id,
        attempt_id: &attempt_id,
        config: Config::dictation(params.language.as_deref()),
    };
    send(&mut socket, &start).await?;
    Ok(socket)
}

async fn session(credentials: &Credentials) -> Result<Session, Failure> {
    match credentials {
        Credentials::AuthFile(path) => {
            let path = match path {
                Some(path) => path.clone(),
                None => codex_auth::auth_path().map_err(|_| Failure::NoSession)?,
            };
            codex_auth::session_for_request(&path)
                .await
                .map_err(|_| Failure::NoSession)
        }
        #[cfg(test)]
        Credentials::Session(session) => Ok(session.clone()),
    }
}

/// The event in a frame, or how the frame ends the session.
pub(super) fn event_of(
    frame: Option<Result<Message, reqwest_websocket::Error>>,
) -> Result<ServerEvent, Failure> {
    match frame {
        Some(Ok(Message::Text(text))) => protocol::parse(&text).map_err(|_| Failure::Unreadable),
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => Ok(ServerEvent::Other),
        Some(Ok(Message::Binary(_))) => Err(Failure::Unreadable),
        Some(Ok(Message::Close { .. }) | Err(_)) | None => Err(Failure::ClosedEarly),
    }
}

async fn send_audio(socket: &mut WebSocket, audio: &Audio, samples: &[f32]) -> Result<(), Failure> {
    send(socket, &audio_message(samples)).await?;
    audio.meter.sent(samples.len());
    Ok(())
}

fn audio_message(samples: &[f32]) -> ClientMessage<'static> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        pcm.extend_from_slice(&to_pcm16(*sample).to_le_bytes());
    }
    ClientMessage::Audio {
        audio: STANDARD.encode(pcm),
    }
}

pub(super) async fn send(
    socket: &mut WebSocket,
    message: &ClientMessage<'_>,
) -> Result<(), Failure> {
    let json = serde_json::to_string(message).map_err(|_| Failure::Unreadable)?;
    socket
        .send(Message::Text(json))
        .await
        .map_err(|_| Failure::ClosedEarly)
}

fn uuid_v4() -> String {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the system random number generator is available");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_version_4_uuids_and_differ() {
        let pattern = regex::Regex::new(
            "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
        )
        .unwrap();
        let (first, second) = (uuid_v4(), uuid_v4());
        assert!(pattern.is_match(&first), "{first}");
        assert!(pattern.is_match(&second), "{second}");
        assert_ne!(first, second);
    }

    #[test]
    fn audio_goes_out_as_base64_of_little_endian_pcm16() {
        let ClientMessage::Audio { audio } = audio_message(&[0.0, 1.0, -1.0, 0.5]) else {
            panic!("an audio message");
        };
        let bytes = STANDARD.decode(audio).unwrap();
        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(samples, [0, i16::MAX, -i16::MAX, to_pcm16(0.5)]);
    }
}
