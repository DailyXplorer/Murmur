//! A dictation stream server for the tests. It speaks the protocol the way
//! the real server was seen to, on a local port, and reports what it heard.

use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tungstenite::handshake::server::{Callback, ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::{Message, WebSocket};

use super::live::{pair, LiveDictation};
use super::session::{Credentials, Params};
use crate::audio_toolkit::FrameTap;
use crate::codex_auth::Session;

pub(crate) enum Reply {
    Event(String),
    /// Drops the connection without a closing handshake.
    Hangup,
}

/// What the server answers at each point of a session.
#[derive(Default)]
pub(crate) struct Script {
    /// The status that answers the handshake instead of 101.
    pub(crate) refuse: Option<u16>,
    /// `session.start` is answered only once this receives, or its sender
    /// is dropped.
    pub(crate) hold_start: Option<mpsc::Receiver<()>>,
    pub(crate) on_start: Vec<Reply>,
    /// Sent after the first audio message.
    pub(crate) on_audio: Vec<Reply>,
    pub(crate) on_close: Vec<Reply>,
}

impl Script {
    /// A session that goes well and whose one utterance is `text`.
    pub(crate) fn transcribing(text: &str) -> Self {
        Self {
            on_start: vec![started()],
            on_close: vec![final_text("utt_1", 1, text), closed()],
            ..Self::default()
        }
    }
}

pub(crate) fn started() -> Reply {
    event(serde_json::json!({
        "type": "session.started",
        "sequence_no": 1,
        "session": {"session_id": "s", "status": "active", "config": {}},
        "frame_rtt_supported": true,
    }))
}

pub(crate) fn closed() -> Reply {
    event(serde_json::json!({
        "type": "session.updated",
        "sequence_no": 9,
        "session": {"session_id": "s", "status": "closed", "config": {}},
    }))
}

pub(crate) fn segment(utterance_id: &str, revision: u64, text: &str) -> Reply {
    event(serde_json::json!({
        "type": "transcript.segment",
        "utterance_id": utterance_id,
        "revision": revision,
        "text": text,
    }))
}

pub(crate) fn final_text(utterance_id: &str, revision: u64, text: &str) -> Reply {
    event(serde_json::json!({
        "type": "transcript.final",
        "utterance_id": utterance_id,
        "revision": revision,
        "text": text,
    }))
}

pub(crate) fn transcript_failed() -> Reply {
    event(serde_json::json!({
        "type": "transcript.failed",
        "utterance_id": null,
        "error": {"code": "failed", "message": "no", "retryable": false},
    }))
}

pub(crate) fn session_error(fatal: bool) -> Reply {
    event(serde_json::json!({
        "type": "session.error",
        "fatal": fatal,
        "error": {"code": "invalid_event", "message": "no", "retryable": false},
    }))
}

pub(crate) fn event(json: serde_json::Value) -> Reply {
    Reply::Event(json.to_string())
}

/// What a client sent, in order.
#[derive(Debug, PartialEq)]
pub(crate) enum Received {
    Start(serde_json::Value),
    /// The samples of one audio message.
    Audio(Vec<i16>),
    Close,
    /// The connection ended.
    Gone,
}

pub(crate) struct FakeServer {
    pub(crate) url: String,
    /// The head of each upgrade request, lowercased, one header a line.
    pub(crate) handshakes: mpsc::Receiver<String>,
    pub(crate) received: mpsc::Receiver<Received>,
}

impl FakeServer {
    pub(super) fn params(&self, language: Option<&str>) -> Params {
        Params {
            url: self.url.clone(),
            credentials: Credentials::Session(Session {
                access_token: "test-token".to_string(),
                account_id: None,
            }),
            language: language.map(str::to_string),
        }
    }

    /// A dictation that streams to this server, with its recorder's tap.
    pub(crate) fn dictation(&self, language: Option<&str>) -> (FrameTap, LiveDictation) {
        let (tap, mut live) = pair();
        live.settle(Ok(self.params(language)));
        (tap, live)
    }

    /// Everything the client sent until its connection ended.
    pub(crate) fn heard(&self) -> Vec<Received> {
        let mut heard = Vec::new();
        loop {
            let received = self
                .received
                .recv_timeout(Duration::from_secs(10))
                .expect("the connection ends");
            let gone = received == Received::Gone;
            heard.push(received);
            if gone {
                return heard;
            }
        }
    }
}

/// Serves `script` to the first connection that is not refused. Every
/// connection's handshake is reported, so a second attempt shows.
pub(crate) fn serve(script: Script) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "ws://{}/backend-api/dictation/stream",
        listener.local_addr().unwrap()
    );
    let (handshakes_tx, handshakes) = mpsc::channel();
    let (received_tx, received) = mpsc::channel();
    thread::spawn(move || {
        let refuse = script.refuse;
        let mut script = Some(script);
        for peer in listener.incoming() {
            let Ok(peer) = peer else { return };
            let handshake = Handshake {
                refuse,
                seen: handshakes_tx.clone(),
            };
            let Ok(socket) = tungstenite::accept_hdr(peer, handshake) else {
                continue;
            };
            if let Some(script) = script.take() {
                converse(socket, &script, &received_tx);
                let _ = received_tx.send(Received::Gone);
            }
        }
    });
    FakeServer {
        url,
        handshakes,
        received,
    }
}

/// Reports the upgrade request, then accepts it as the real server does or
/// refuses it.
struct Handshake {
    refuse: Option<u16>,
    seen: mpsc::Sender<String>,
}

impl Callback for Handshake {
    fn on_request(
        self,
        request: &Request,
        mut response: Response,
    ) -> Result<Response, ErrorResponse> {
        let head: Vec<String> = request
            .headers()
            .iter()
            .map(|(name, value)| {
                format!("{name}: {}", value.to_str().unwrap_or_default()).to_lowercase()
            })
            .collect();
        let _ = self.seen.send(head.join("\n"));
        if let Some(status) = self.refuse {
            let mut refusal = ErrorResponse::new(None);
            *refusal.status_mut() = StatusCode::from_u16(status).unwrap();
            return Err(refusal);
        }
        response.headers_mut().insert(
            "sec-websocket-protocol",
            "chatgpt-dictation".parse().unwrap(),
        );
        Ok(response)
    }
}

fn converse(mut socket: WebSocket<TcpStream>, script: &Script, received: &mpsc::Sender<Received>) {
    let mut audio_heard = false;
    loop {
        let text = match socket.read() {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => return,
            Ok(_) => continue,
        };
        let message: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
        let replies: &[Reply] = match message["type"].as_str().unwrap() {
            "session.start" => {
                let _ = received.send(Received::Start(message));
                if let Some(hold) = &script.hold_start {
                    let _ = hold.recv();
                }
                &script.on_start
            }
            "audio.append" => {
                let pcm = STANDARD.decode(message["audio"].as_str().unwrap()).unwrap();
                let samples = pcm
                    .chunks_exact(2)
                    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                let _ = received.send(Received::Audio(samples));
                if std::mem::replace(&mut audio_heard, true) {
                    &[]
                } else {
                    &script.on_audio
                }
            }
            "session.close" => {
                let _ = received.send(Received::Close);
                &script.on_close
            }
            other => panic!("a client message of type {other}"),
        };
        for reply in replies {
            match reply {
                Reply::Event(json) => {
                    if socket.send(Message::text(json.as_str())).is_err() {
                        return;
                    }
                }
                Reply::Hangup => return,
            }
        }
    }
}
