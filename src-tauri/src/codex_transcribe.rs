use anyhow::{anyhow, Context, Result};
use hound::WavSpec;
use log::{debug, info};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const TRANSCRIBE_URL: &str = "https://chatgpt.com/backend-api/transcribe";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct CodexAuthStatus {
    pub signed_in: bool,
}

#[derive(Debug, Deserialize)]
struct AuthFile {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    #[serde(default)]
    tokens: Option<AuthTokens>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AuthTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
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

pub fn transcribe(samples: &[f32], language: Option<&str>) -> Result<String> {
    if samples.is_empty() {
        return Ok(String::new());
    }

    let wav = pcm_f32_to_wav_bytes(samples)?;
    let session = load_session()?;
    match transcribe_with_session(&session, &wav, language) {
        Ok(text) => Ok(text),
        Err(err) if is_unauthorized(&err) => {
            info!("Codex transcription got an unauthorized response, refreshing session");
            let refreshed = refresh_session(&session)?;
            transcribe_with_session(&refreshed, &wav, language)
        }
        Err(err) => Err(err),
    }
}

fn is_unauthorized(err: &anyhow::Error) -> bool {
    err.to_string().contains("401")
}

fn transcribe_with_session(
    session: &Session,
    wav: &[u8],
    language: Option<&str>,
) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(user_agent())
        .build()
        .context("failed to build transcription HTTP client")?;

    let mut form = reqwest::blocking::multipart::Form::new().part(
        "file",
        reqwest::blocking::multipart::Part::bytes(wav.to_vec())
            .file_name("codex.wav")
            .mime_str("audio/wav")
            .context("invalid wav content type")?,
    );

    if let Some(language) = language.filter(|code| !code.is_empty() && *code != "auto") {
        form = form.text("language", api_language(language).to_string());
    }

    let mut request = client
        .post(TRANSCRIBE_URL)
        .header("Authorization", format!("Bearer {}", session.access_token))
        .header("originator", "Codex Desktop")
        .header("OAI-Product-Sku", "CODEX")
        .multipart(form);

    if let Some(account_id) = session.account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request
        .send()
        .context("Codex transcription request failed")?;
    let status = response.status();
    let body = response
        .text()
        .context("failed to read Codex transcription response")?;

    if !status.is_success() {
        let preview: String = body.chars().take(300).collect();
        return Err(anyhow!("Codex transcription failed ({status}): {preview}"));
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
    let auth: AuthFile =
        serde_json::from_str(&raw).context("failed to parse ~/.codex/auth.json")?;

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

    let _ = auth.openai_api_key;
    Err(anyhow!(
        "Codex/ChatGPT session is missing. Open the Codex app and sign in, then try again."
    ))
}

fn refresh_session(current: &Session) -> Result<Session> {
    let path = auth_path()?;
    let raw = fs::read_to_string(&path).context("failed to read Codex auth.json for refresh")?;
    let mut auth: serde_json::Value =
        serde_json::from_str(&raw).context("failed to parse Codex auth.json for refresh")?;

    let refresh_token = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("refresh_token"))
        .and_then(|value| value.as_str())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow!("Codex session has no refresh token"))?
        .to_string();

    let client_id = jwt_claim(&current.access_token, "client_id")
        .ok_or_else(|| anyhow!("Codex access token is missing client_id"))?;

    let client = reqwest::blocking::Client::builder()
        .user_agent(user_agent())
        .build()
        .context("failed to build auth refresh client")?;

    let response = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "client_id": client_id,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "scope": "openid profile email"
        }))
        .send()
        .context("Codex token refresh request failed")?;

    let status = response.status();
    let body = response
        .text()
        .context("failed to read token refresh body")?;
    if !status.is_success() {
        let preview: String = body.chars().take(200).collect();
        return Err(anyhow!("Codex token refresh failed ({status}): {preview}"));
    }

    let parsed: RefreshResponse =
        serde_json::from_str(&body).context("failed to parse Codex token refresh response")?;
    let access_token = parsed
        .access_token
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow!("Codex token refresh returned no access token"))?;

    if let Some(tokens) = auth
        .get_mut("tokens")
        .and_then(|value| value.as_object_mut())
    {
        tokens.insert(
            "access_token".to_string(),
            serde_json::Value::String(access_token.clone()),
        );
        if let Some(refresh_token) = parsed.refresh_token.filter(|token| !token.is_empty()) {
            tokens.insert(
                "refresh_token".to_string(),
                serde_json::Value::String(refresh_token),
            );
        }
        if let Some(id_token) = parsed.id_token.filter(|token| !token.is_empty()) {
            tokens.insert("id_token".to_string(), serde_json::Value::String(id_token));
        }
    }
    auth.as_object_mut().map(|object| {
        object.insert(
            "last_refresh".to_string(),
            serde_json::Value::String(now_rfc3339()),
        )
    });

    let serialized =
        serde_json::to_string_pretty(&auth).context("failed to serialize refreshed Codex auth")?;
    write_auth_file(&path, &serialized)?;
    debug!("Wrote refreshed Codex session");

    let account_id = current
        .account_id
        .clone()
        .or_else(|| chatgpt_account_id_from_jwt(&access_token));

    Ok(Session {
        access_token,
        account_id,
    })
}

fn write_auth_file(path: &PathBuf, contents: &str) -> Result<()> {
    fs::write(path, contents).context("failed to write refreshed Codex auth.json")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn auth_path() -> Result<PathBuf> {
    let home = dirs_next_home().ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(home.join(".codex").join("auth.json"))
}

fn dirs_next_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn user_agent() -> String {
    let os = if cfg!(target_os = "macos") {
        "Mac OS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    format!("Codex Desktop/26.818.32112 ({os}; {arch})")
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

fn jwt_claim(token: &str, claim: &str) -> Option<String> {
    jwt_payload(token)?
        .get(claim)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn jwt_payload(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = b64url_decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn b64url_decode(input: &str) -> Result<Vec<u8>> {
    let mut padded = input.replace('-', "+").replace('_', "/");
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .context("invalid base64url")
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn encode_wav_has_header_and_samples() {
        let wav = pcm_f32_to_wav_bytes(&[0.5; 160]).unwrap();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
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
        let result = transcribe(&samples, Some("fr"));
        assert!(
            result.is_ok(),
            "Codex transcription failed: {:?}",
            result.err()
        );
    }
}
