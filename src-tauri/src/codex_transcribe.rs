use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE, Engine as _};
use hound::WavSpec;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::Duration;

use crate::ProcessingOperation;

const TRANSCRIBE_URL: &str = "https://chatgpt.com/backend-api/transcribe";
const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CodexAuthStatus {
    pub signed_in: bool,
}

#[derive(Debug, Deserialize)]
struct AuthFile {
    #[serde(default)]
    tokens: Option<AuthTokens>,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthTokens {
    access_token: String,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TranscribeJson {
    text: Option<String>,
    transcript: Option<String>,
}

struct Session {
    access_token: String,
    account_id: Option<String>,
}

pub fn auth_status() -> CodexAuthStatus {
    CodexAuthStatus {
        signed_in: load_session().is_ok(),
    }
}

pub async fn transcribe(
    samples: &[f32],
    language: Option<&str>,
    operation: &ProcessingOperation,
) -> Result<String> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    if operation.is_cancelled() {
        return Err(anyhow!("Codex transcription cancelled"));
    }

    let wav = pcm_f32_to_wav_bytes(samples)?;
    if operation.is_cancelled() {
        return Err(anyhow!("Codex transcription cancelled"));
    }
    let session = load_session()?;
    if operation.is_cancelled() {
        return Err(anyhow!("Codex transcription cancelled"));
    }
    transcribe_with_session(&session, &wav, language, operation).await
}

async fn transcribe_with_session(
    session: &Session,
    wav: &[u8],
    language: Option<&str>,
    operation: &ProcessingOperation,
) -> Result<String> {
    transcribe_with_session_at(TRANSCRIBE_URL, session, wav, language, operation).await
}

async fn transcribe_with_session_at(
    url: &str,
    session: &Session,
    wav: &[u8],
    language: Option<&str>,
    operation: &ProcessingOperation,
) -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .build()
        .context("failed to build transcription HTTP client")?;

    let mut form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(wav.to_vec())
            .file_name("codex.wav")
            .mime_str("audio/wav")
            .context("invalid wav content type")?,
    );

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
        _ = operation.cancelled() => return Err(anyhow!("Codex transcription cancelled")),
        response = request.send() => response.context("Codex transcription request failed")?,
    };
    let status = response.status();
    let body = tokio::select! {
        _ = operation.cancelled() => return Err(anyhow!("Codex transcription cancelled")),
        body = response.text() => body.context("failed to read Codex transcription response")?,
    };

    if !status.is_success() {
        return if status.as_u16() == 401 {
            Err(anyhow!(
                "Codex transcription session expired. Open Codex, sign in again, and retry."
            ))
        } else {
            Err(anyhow!("Codex transcription failed with HTTP {status}"))
        };
    }

    parse_transcript(&body)
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

fn load_session() -> Result<Session> {
    let path = auth_path()?;
    let raw = fs::read_to_string(&path).with_context(|| {
        format!(
            "Codex/ChatGPT session not found at {}. Open the Codex app and sign in.",
            path.display()
        )
    })?;
    let auth = parse_auth_file(&raw, &path)?;

    if let Some(tokens) = auth.tokens.filter(|tokens| !tokens.access_token.is_empty()) {
        let account_id = tokens
            .account_id
            .clone()
            .or_else(|| chatgpt_account_id_from_jwt(&tokens.access_token));
        return Ok(Session {
            access_token: tokens.access_token,
            account_id,
        });
    }

    Err(anyhow!(
        "A ChatGPT subscription session is required. Sign in to Codex with ChatGPT and configure Codex to store credentials in auth.json."
    ))
}

fn parse_auth_file(raw: &str, path: &std::path::Path) -> Result<AuthFile> {
    serde_json::from_str(raw).with_context(|| format!("failed to parse {}", path.display()))
}

fn auth_path() -> Result<PathBuf> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("auth.json"));
    }
    let home = user_home().ok_or_else(|| anyhow!("User home directory is not set"))?;
    Ok(home.join(".codex").join("auth.json"))
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn user_agent() -> String {
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

fn chatgpt_account_id_from_jwt(token: &str) -> Option<String> {
    jwt_payload(token)?
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn jwt_payload(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = decode_base64url(payload)?;
    serde_json::from_slice(&decoded).ok()
}

fn decode_base64url(input: &str) -> Option<Vec<u8>> {
    let padding_start = input.find('=').unwrap_or(input.len());
    let padding = &input[padding_start..];
    let remainder = input.len() % 4;

    if !padding.bytes().all(|byte| byte == b'=') || padding.len() > 2 {
        return None;
    }
    if !padding.is_empty() && remainder != 0 {
        return None;
    }
    if padding.is_empty() && remainder == 1 {
        return None;
    }

    let mut normalized = input.to_string();
    if padding.is_empty() {
        normalized.extend(std::iter::repeat_n('=', (4 - remainder) % 4));
    }

    URL_SAFE.decode(normalized).ok()
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
    fn extracts_chatgpt_account_id_from_jwt() {
        let token = "header.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8xMjMifX0.signature";
        assert_eq!(
            chatgpt_account_id_from_jwt(token).as_deref(),
            Some("acct_123")
        );
    }

    #[test]
    fn decodes_padded_and_unpadded_base64url() {
        assert_eq!(decode_base64url("SGVsbG8"), Some(b"Hello".to_vec()));
        assert_eq!(decode_base64url("SGVsbG8="), Some(b"Hello".to_vec()));
        assert_eq!(decode_base64url("SGk="), Some(b"Hi".to_vec()));
        assert_eq!(decode_base64url("SGk"), Some(b"Hi".to_vec()));
    }

    #[test]
    fn rejects_malformed_base64url_padding() {
        for malformed in [
            "SG=VsbG8",
            "SGVsbG8===",
            "SGVsbG8=A",
            "TQ=",
            "TQ===",
            "TQ==A",
            "A",
            "SGVsbG8==",
        ] {
            assert_eq!(decode_base64url(malformed), None, "accepted {malformed}");
        }
    }

    #[test]
    fn auth_parse_error_names_the_resolved_path() {
        let path = std::path::Path::new("/custom/codex/auth.json");
        let error = parse_auth_file("{", path).unwrap_err().to_string();
        assert!(error.contains("/custom/codex/auth.json"));
    }

    #[test]
    fn encode_wav_has_header_and_samples() {
        let wav = pcm_f32_to_wav_bytes(&[0.5; 160]).unwrap();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
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
            transcribe_with_session_at(&endpoint, &session, b"test wav", None, &request_operation)
                .await
        });

        accepted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(operation.cancel());
        let result = tauri::async_runtime::block_on(request).unwrap();
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert!(closed_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        server.join().unwrap();
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
