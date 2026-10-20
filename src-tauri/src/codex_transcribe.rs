use anyhow::{anyhow, Context, Result};
use hound::WavSpec;
use log::warn;
use serde::Deserialize;
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use crate::codex_auth::{self, Session};
use crate::transcription_error::{retry_delay, FailureKind, TranscriptionError};
use crate::ProcessingOperation;

pub use crate::codex_auth::CodexAuthStatus;

pub(crate) const TRANSCRIBE_URL: &str = "https://chatgpt.com/backend-api/transcribe";
const SAMPLE_RATE: u32 = 16_000;
const WAV_BYTES_PER_SAMPLE: usize = 2;

/// The Codex CLI only ever uploaded `audio/wav` to this endpoint, so Murmur
/// sends uncompressed WAV. The endpoint publishes no size limit; the public
/// OpenAI transcription API caps uploads at 25 MB (about 13.6 minutes here),
/// so longer recordings get a warning before upload.
pub const UPLOAD_SOFT_LIMIT_BYTES: usize = 25 * 1024 * 1024;

/// Covers connect, upload and response. Long recordings get one extra second
/// per 64 KiB, which assumes at least ~512 kbit/s of upstream bandwidth,
/// shared by the uploads that run at once.
const BASE_REQUEST_TIMEOUT_SECS: u64 = 60;
const TIMEOUT_BYTES_PER_EXTRA_SECOND: u64 = 64 * 1024;
/// For the TCP and TLS handshakes: about three round trips, so a link of a
/// second or more of round-trip time still connects.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct TranscribeJson {
    text: Option<String>,
    transcript: Option<String>,
}

enum RequestError {
    Cancelled,
    Failed(TranscriptionError),
}

pub fn auth_status() -> CodexAuthStatus {
    codex_auth::auth_status()
}

pub fn upload_size_for_samples(sample_count: usize) -> usize {
    44 + sample_count * WAV_BYTES_PER_SAMPLE
}

fn request_timeout(upload_bytes: usize, uploads_at_once: usize) -> Duration {
    let bytes_on_the_uplink = upload_bytes as u64 * uploads_at_once.max(1) as u64;
    Duration::from_secs(
        BASE_REQUEST_TIMEOUT_SECS + bytes_on_the_uplink / TIMEOUT_BYTES_PER_EXTRA_SECOND,
    )
}

pub async fn transcribe(
    samples: &[f32],
    language: Option<&str>,
    operation: &ProcessingOperation,
) -> Result<String> {
    transcribe_at(TRANSCRIBE_URL, None, samples, language, 1, operation).await
}

/// `transcribe` against `url`, with the session in `auth_path` (the Codex
/// default when `None`), while `uploads_at_once` requests share the uplink.
pub(crate) async fn transcribe_at(
    url: &str,
    auth_path: Option<&Path>,
    samples: &[f32],
    language: Option<&str>,
    uploads_at_once: usize,
    operation: &ProcessingOperation,
) -> Result<String> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    if operation.is_cancelled() {
        return Err(anyhow!("Codex transcription cancelled"));
    }

    let wav = pcm_f32_to_wav_bytes(samples)?;
    let timeout = request_timeout(wav.len(), uploads_at_once);
    let auth_path = match auth_path {
        Some(path) => path.to_path_buf(),
        None => codex_auth::auth_path()?,
    };
    // A refresh under way may hold the session for a while. Dropping the wait
    // is safe: the refresh runs detached and still saves its tokens.
    let mut session = tokio::select! {
        _ = operation.cancelled() => return Err(anyhow!("Codex transcription cancelled")),
        session = codex_auth::session_for_request(&auth_path) => session?,
    };
    let mut recovered_from_unauthorized = false;
    let mut retries = 0;

    loop {
        if operation.is_cancelled() {
            return Err(anyhow!("Codex transcription cancelled"));
        }
        let result =
            transcribe_with_session_at(url, &session, &wav, language, timeout, operation).await;
        let error = match result {
            Ok(text) => return Ok(text),
            Err(RequestError::Cancelled) => return Err(anyhow!("Codex transcription cancelled")),
            Err(RequestError::Failed(error)) => error,
        };

        // Like Codex: on the first 401, refresh once and resend once.
        if error.kind == FailureKind::SessionExpired && !recovered_from_unauthorized {
            recovered_from_unauthorized = true;
            session = tokio::select! {
                _ = operation.cancelled() => return Err(anyhow!("Codex transcription cancelled")),
                session = codex_auth::recover_from_unauthorized(&auth_path, &session) => session?,
            };
            continue;
        }

        let Some(delay) = retry_delay(error.kind, retries) else {
            return Err(error.into());
        };
        retries += 1;
        warn!(
            "Codex transcription attempt failed ({:?}); retrying in {} ms",
            error.kind,
            delay.as_millis()
        );
        tokio::select! {
            _ = operation.cancelled() => return Err(anyhow!("Codex transcription cancelled")),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

async fn transcribe_with_session_at(
    url: &str,
    session: &Session,
    wav: &[u8],
    language: Option<&str>,
    timeout: Duration,
    operation: &ProcessingOperation,
) -> Result<String, RequestError> {
    let failed = |kind, detail: String| RequestError::Failed(TranscriptionError::new(kind, detail));
    let client = reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .build()
        .map_err(|error| {
            failed(
                FailureKind::Other,
                format!("failed to build transcription HTTP client: {error}"),
            )
        })?;

    let part = reqwest::multipart::Part::bytes(wav.to_vec())
        .file_name("codex.wav")
        .mime_str("audio/wav")
        .map_err(|error| {
            failed(
                FailureKind::Other,
                format!("invalid wav content type: {error}"),
            )
        })?;
    let mut form = reqwest::multipart::Form::new().part("file", part);

    if let Some(language) = language.filter(|code| !code.is_empty() && *code != "auto") {
        form = form.text("language", api_language(language).to_string());
    }

    let mut request = client
        .post(url)
        .header("Authorization", format!("Bearer {}", session.access_token))
        .header("originator", "Codex Desktop")
        .header("OAI-Product-Sku", "CODEX")
        .multipart(form);

    if let Some(account_id) = session.account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = tokio::select! {
        _ = operation.cancelled() => return Err(RequestError::Cancelled),
        response = request.send() => response.map_err(|error| failed(
            FailureKind::from_transport(&error),
            format!("Codex transcription request failed: {error}"),
        ))?,
    };
    let status = response.status();
    let body = tokio::select! {
        _ = operation.cancelled() => return Err(RequestError::Cancelled),
        body = response.text() => body.map_err(|error| failed(
            FailureKind::from_transport(&error),
            format!("failed to read Codex transcription response: {error}"),
        ))?,
    };

    if !status.is_success() {
        let kind = FailureKind::from_status(status.as_u16());
        let excerpt: String = body.chars().take(200).collect();
        return Err(failed(
            kind,
            format!("Codex transcription failed with HTTP {status}: {excerpt}"),
        ));
    }

    parse_transcript(&body).map_err(|error| failed(FailureKind::Other, error.to_string()))
}

fn parse_transcript(body: &str) -> Result<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if let Ok(parsed) = serde_json::from_str::<TranscribeJson>(trimmed) {
        if let Some(text) = parsed.text.or(parsed.transcript) {
            return Ok(text.trim().to_string());
        }
    }
    if trimmed.starts_with('{') {
        return Err(anyhow!(
            "Codex transcription returned JSON without a transcript field"
        ));
    }
    Ok(trimmed.to_string())
}

fn api_language(language: &str) -> &str {
    match language {
        "zh-Hans" | "zh-Hant" => "zh",
        other => other.split('-').next().unwrap_or(other),
    }
}

pub(crate) fn user_agent() -> String {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    format!("Murmur/{} (Mac OS; {arch})", env!("CARGO_PKG_VERSION"))
}

fn pcm_f32_to_wav_bytes(samples: &[f32]) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .context("failed to create in-memory WAV writer")?;
        for sample in samples {
            let clipped = sample.clamp(-1.0, 1.0);
            writer
                .write_sample((clipped * i16::MAX as f32) as i16)
                .context("failed to write WAV sample")?;
        }
        writer.finalize().context("failed to finalize WAV")?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OperationId, ProcessingOperation};
    use std::io::Read;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn parse_json_text_field() {
        assert_eq!(
            parse_transcript(r#"{"text":"Bonjour"}"#).unwrap(),
            "Bonjour"
        );
    }

    #[test]
    fn parse_json_transcript_field() {
        assert_eq!(
            parse_transcript(r#"{"transcript":"Hello"}"#).unwrap(),
            "Hello"
        );
    }

    #[test]
    fn parse_plain_text_response() {
        assert_eq!(
            parse_transcript("  Bonjour le monde  ").unwrap(),
            "Bonjour le monde"
        );
    }

    #[test]
    fn reject_json_without_transcript() {
        assert!(parse_transcript(r#"{"status":"ok"}"#).is_err());
    }

    #[test]
    fn normalizes_regional_language_for_request() {
        assert_eq!(api_language("fr-FR"), "fr");
        assert_eq!(api_language("zh-Hant"), "zh");
    }

    #[test]
    fn encode_wav_has_header_and_samples() {
        let wav = pcm_f32_to_wav_bytes(&[0.5; 160]).unwrap();
        assert_eq!(wav.len(), upload_size_for_samples(160));
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn timeout_scales_with_upload_size() {
        assert_eq!(request_timeout(100_000, 1), Duration::from_secs(61));
        let fifteen_minutes = upload_size_for_samples(15 * 60 * SAMPLE_RATE as usize);
        assert_eq!(
            request_timeout(fifteen_minutes, 1),
            Duration::from_secs(499)
        );
    }

    #[test]
    fn parallel_uploads_each_get_their_share_of_the_uplink() {
        let five_minutes = upload_size_for_samples(5 * 60 * SAMPLE_RATE as usize);
        let alone = request_timeout(five_minutes, 1);
        let shared = request_timeout(five_minutes, 3);
        assert_eq!(alone, Duration::from_secs(206));
        // Three at once on the same uplink take three times as long to send.
        assert_eq!(shared, Duration::from_secs(499));
        assert_eq!(request_timeout(five_minutes, 0), alone);
    }

    #[test]
    fn soft_limit_is_reached_after_about_thirteen_minutes() {
        let samples_per_minute = 60 * SAMPLE_RATE as usize;
        assert!(upload_size_for_samples(13 * samples_per_minute) < UPLOAD_SOFT_LIMIT_BYTES);
        assert!(upload_size_for_samples(14 * samples_per_minute) > UPLOAD_SOFT_LIMIT_BYTES);
    }

    fn reply_once(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            use std::io::Write;
            let (mut peer, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 8192];
            let _ = peer.read(&mut buffer);
            let body = r#"{"detail":"nope"}"#;
            let _ = write!(
                peer,
                "HTTP/1.1 {status_line}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        endpoint
    }

    fn failure_kind_for(status_line: &'static str) -> FailureKind {
        let endpoint = reply_once(status_line);
        let session = Session {
            access_token: "test-token".to_string(),
            account_id: None,
        };
        let operation = ProcessingOperation::new(OperationId(3));
        match tauri::async_runtime::block_on(transcribe_with_session_at(
            &endpoint,
            &session,
            b"wav",
            None,
            request_timeout(3, 1),
            &operation,
        )) {
            Err(RequestError::Failed(error)) => error.kind,
            _ => panic!("{status_line} must fail"),
        }
    }

    #[test]
    fn http_failures_are_classified() {
        assert_eq!(
            failure_kind_for("401 Unauthorized"),
            FailureKind::SessionExpired
        );
        assert_eq!(
            failure_kind_for("429 Too Many Requests"),
            FailureKind::RateLimited
        );
        assert_eq!(
            failure_kind_for("502 Bad Gateway"),
            FailureKind::ServerError
        );
        assert_eq!(
            failure_kind_for("413 Payload Too Large"),
            FailureKind::TooLarge
        );
    }

    #[test]
    fn unreachable_service_is_a_network_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let session = Session {
            access_token: "test-token".to_string(),
            account_id: None,
        };
        let operation = ProcessingOperation::new(OperationId(4));
        match tauri::async_runtime::block_on(transcribe_with_session_at(
            &endpoint,
            &session,
            b"wav",
            None,
            request_timeout(3, 1),
            &operation,
        )) {
            Err(RequestError::Failed(error)) => assert_eq!(error.kind, FailureKind::Network),
            _ => panic!("a closed port must fail"),
        }
    }

    #[test]
    fn cancellation_closes_a_stalled_codex_http_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = peer.read(&mut buffer);
            accepted_tx.send(()).unwrap();
            loop {
                match peer.read(&mut buffer) {
                    Ok(0) => {
                        closed_tx.send(true).unwrap();
                        return;
                    }
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) =>
                    {
                        closed_tx.send(false).unwrap();
                        return;
                    }
                    Err(error) => panic!("stalled peer read failed: {error}"),
                }
            }
        });

        let operation = ProcessingOperation::new(OperationId(7));
        let session = Session {
            access_token: "test-token".to_string(),
            account_id: None,
        };
        let request_operation = operation.clone();
        let request = tauri::async_runtime::spawn(async move {
            transcribe_with_session_at(
                &endpoint,
                &session,
                b"test wav",
                None,
                request_timeout(8, 1),
                &request_operation,
            )
            .await
        });

        accepted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(operation.cancel());
        let result = tauri::async_runtime::block_on(request).unwrap();
        assert!(matches!(result, Err(RequestError::Cancelled)));
        assert!(closed_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        server.join().unwrap();
    }

    #[test]
    fn cancellation_stops_waiting_for_a_token_refresh() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        // Expires within the refresh window: the request refreshes first.
        let expiring = format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(
                serde_json::json!({"exp": chrono::Utc::now().timestamp() + 60}).to_string()
            )
        );
        let auth = |access_token: &str| {
            serde_json::json!({"tokens": {"access_token": access_token, "refresh_token": "rt"}})
                .to_string()
        };
        std::fs::write(&path, auth(&expiring)).unwrap();
        let operation = ProcessingOperation::new(OperationId(8));
        let request_operation = operation.clone();
        let request_path = path.clone();

        tauri::async_runtime::block_on(async {
            // Another refresh is under way.
            let other_refresh = codex_auth::REFRESH_LOCK.lock().await;
            let request = tauri::async_runtime::spawn(async move {
                transcribe_at(
                    "http://127.0.0.1:9",
                    Some(&request_path),
                    &[0.1; 160],
                    None,
                    1,
                    &request_operation,
                )
                .await
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(operation.cancel());
            let result = tokio::time::timeout(Duration::from_secs(1), request)
                .await
                .expect("a cancel must not wait for the refresh")
                .unwrap();
            assert!(result.unwrap_err().to_string().contains("cancelled"));
            // The other refresh saved new tokens, so the queued one ends offline.
            std::fs::write(&path, auth("fresh")).unwrap();
            drop(other_refresh);
        });
        assert!(codex_auth::wait_for_refreshes(Duration::from_secs(5)));
    }

    #[test]
    #[ignore]
    fn live_codex_session_transcribes() {
        let sample_rate = SAMPLE_RATE as f32;
        let samples: Vec<f32> = (0..SAMPLE_RATE)
            .map(|i| {
                let t = i as f32 / sample_rate;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.2
            })
            .collect();
        let operation = ProcessingOperation::new(OperationId(1));
        let result = tauri::async_runtime::block_on(transcribe(&samples, Some("fr"), &operation));
        assert!(
            result.is_ok(),
            "Codex transcription failed: {:?}",
            result.err()
        );
    }
}
