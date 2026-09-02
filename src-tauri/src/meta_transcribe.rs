use crate::audio_toolkit::pcm_f32_to_wav_bytes;
use anyhow::{anyhow, Context, Result};
use reqwest::StatusCode;
use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::time::Duration;

const TRANSCRIBE_URL: &str = "https://api.meta.ai/v1/asr/transcribe";
const MODEL: &str = "muse-voice-transcribe-1.0";
const SAMPLE_RATE: usize = 16_000;
const MAX_AUDIO_SAMPLES: usize = SAMPLE_RATE * 60 * 10;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const KEYCHAIN_SERVICE: &str = "com.dailyxplorer.murmur.meta-model-api";
const KEYCHAIN_ACCOUNT: &str = "default";
const KEYCHAIN_ITEM_NOT_FOUND: i32 = -25_300;

#[derive(Debug, Clone, Serialize, Type)]
pub struct MetaApiStatus {
    pub configured: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetaTranscribeRequest {
    model: &'static str,
    audio_encoding: &'static str,
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    language_bias: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    keywords: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetaTranscribeResponse {
    transcript: String,
}

#[derive(Debug, Deserialize)]
struct MetaErrorResponse {
    message: Option<String>,
    error: Option<MetaErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct MetaErrorDetail {
    message: Option<String>,
}

pub fn status() -> MetaApiStatus {
    MetaApiStatus {
        configured: load_api_key().is_ok(),
    }
}

pub fn save_api_key(api_key: &str) -> Result<()> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(anyhow!("Enter a Meta Model API key before saving."));
    }
    if api_key.len() > 4096 {
        return Err(anyhow!("The Meta Model API key is unexpectedly long."));
    }

    set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, api_key.as_bytes())
        .context("failed to save the Meta Model API key in macOS Keychain")
}

pub fn clear_api_key() -> Result<()> {
    match delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == KEYCHAIN_ITEM_NOT_FOUND => Ok(()),
        Err(error) => {
            Err(error).context("failed to remove the Meta Model API key from macOS Keychain")
        }
    }
}

pub fn transcribe(samples: &[f32], language: Option<&str>, keywords: &[String]) -> Result<String> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    validate_audio_length(samples.len())?;

    let request = build_request(language, keywords);
    let wav = pcm_f32_to_wav_bytes(samples)?;
    let api_key = load_api_key()?;
    transcribe_with_key(&api_key, &wav, &request)
}

fn validate_audio_length(sample_count: usize) -> Result<()> {
    if sample_count > MAX_AUDIO_SAMPLES {
        return Err(anyhow!(
            "Meta accepts recordings up to 10 minutes. Shorten this recording and retry."
        ));
    }
    Ok(())
}

fn build_request(language: Option<&str>, keywords: &[String]) -> MetaTranscribeRequest {
    MetaTranscribeRequest {
        model: MODEL,
        audio_encoding: "WAV",
        mode: "PUSH_TO_TALK",
        language_bias: language.and_then(meta_language).map(|name| vec![name]),
        keywords: keywords
            .iter()
            .map(|keyword| keyword.trim())
            .filter(|keyword| !keyword.is_empty())
            .map(str::to_owned)
            .collect(),
    }
}

fn transcribe_with_key(
    api_key: &str,
    wav: &[u8],
    request: &MetaTranscribeRequest,
) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(Duration::from_secs(10))
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("failed to build Meta transcription HTTP client")?;
    let request_json = serde_json::to_string(request)
        .context("failed to serialize the Meta transcription request")?;
    let form = reqwest::blocking::multipart::Form::new()
        .part(
            "request",
            reqwest::blocking::multipart::Part::text(request_json)
                .mime_str("application/json")
                .context("invalid Meta request content type")?,
        )
        .part(
            "audio",
            reqwest::blocking::multipart::Part::bytes(wav.to_vec())
                .file_name("murmur.wav")
                .mime_str("audio/wav")
                .context("invalid WAV content type")?,
        );

    let response = client
        .post(TRANSCRIBE_URL)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .context("Meta transcription request failed")?;
    let status = response.status();
    let body = response
        .text()
        .context("failed to read the Meta transcription response")?;

    if !status.is_success() {
        return Err(meta_http_error(status, &body));
    }

    let parsed: MetaTranscribeResponse =
        serde_json::from_str(&body).context("Meta transcription returned an invalid response")?;
    Ok(parsed.transcript.trim().to_string())
}

fn load_api_key() -> Result<String> {
    let bytes = get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|error| {
        if error.code() == KEYCHAIN_ITEM_NOT_FOUND {
            anyhow!("A Meta Model API key is required. Add one in Murmur's transcription settings.")
        } else {
            anyhow!(error).context("failed to read the Meta Model API key from macOS Keychain")
        }
    })?;
    let key = String::from_utf8(bytes).context("the Meta Model API key is not valid UTF-8")?;
    let key = key.trim();
    if key.is_empty() {
        return Err(anyhow!(
            "A Meta Model API key is required. Add one in Murmur's transcription settings."
        ));
    }
    Ok(key.to_owned())
}

fn meta_http_error(status: StatusCode, body: &str) -> anyhow::Error {
    let detail = serde_json::from_str::<MetaErrorResponse>(body)
        .ok()
        .and_then(|error| {
            error
                .message
                .or_else(|| error.error.and_then(|item| item.message))
        })
        .filter(|message| !message.trim().is_empty());

    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => anyhow!(
            "Meta rejected the Model API key. Replace it in Murmur's transcription settings."
        ),
        StatusCode::TOO_MANY_REQUESTS => {
            anyhow!("Meta transcription is temporarily rate limited. Wait a moment and retry.")
        }
        _ => match detail {
            Some(message) => anyhow!("Meta transcription failed with HTTP {status}: {message}"),
            None => anyhow!("Meta transcription failed with HTTP {status}"),
        },
    }
}

fn meta_language(language: &str) -> Option<&'static str> {
    match language.split('-').next().unwrap_or(language) {
        "ar" => Some("Arabic"),
        "bn" => Some("Bengali"),
        "nl" => Some("Dutch"),
        "en" => Some("English"),
        "fr" => Some("French"),
        "de" => Some("German"),
        "he" => Some("Hebrew"),
        "hi" => Some("Hindi"),
        "id" => Some("Indonesian"),
        "it" => Some("Italian"),
        "ja" => Some("Japanese"),
        "kn" => Some("Kannada"),
        "ko" => Some("Korean"),
        "ms" => Some("Malay"),
        "zh" => Some("Mandarin Chinese"),
        "mr" => Some("Marathi"),
        "pl" => Some("Polish"),
        "pt" => Some("Portuguese"),
        "es" => Some("Spanish"),
        "fil" | "tl" => Some("Tagalog"),
        "ta" => Some("Tamil"),
        "te" => Some("Telugu"),
        "th" => Some("Thai"),
        "tr" => Some("Turkish"),
        "vi" => Some("Vietnamese"),
        _ => None,
    }
}

fn user_agent() -> String {
    format!("Murmur/{} (Mac OS)", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_matches_meta_file_api_contract() {
        let request = build_request(Some("fr-FR"), &[" Murmur ".to_string(), "".to_string()]);
        let value = serde_json::to_value(request).unwrap();

        assert_eq!(value["model"], MODEL);
        assert_eq!(value["audioEncoding"], "WAV");
        assert_eq!(value["mode"], "PUSH_TO_TALK");
        assert_eq!(value["languageBias"], serde_json::json!(["French"]));
        assert_eq!(value["keywords"], serde_json::json!(["Murmur"]));
    }

    #[test]
    fn auto_or_unsupported_languages_leave_detection_to_meta() {
        assert!(build_request(None, &[]).language_bias.is_none());
        assert!(build_request(Some("sv"), &[]).language_bias.is_none());
    }

    #[test]
    fn rejects_audio_over_meta_file_limit_before_loading_credentials() {
        let error = validate_audio_length(MAX_AUDIO_SAMPLES + 1)
            .unwrap_err()
            .to_string();
        assert!(error.contains("up to 10 minutes"));
    }

    #[test]
    fn regional_chinese_maps_to_mandarin() {
        assert_eq!(meta_language("zh-Hant"), Some("Mandarin Chinese"));
    }

    #[test]
    fn authentication_errors_do_not_echo_the_response_body() {
        let error = meta_http_error(
            StatusCode::UNAUTHORIZED,
            r#"{"message":"secret request detail"}"#,
        );
        assert!(!error.to_string().contains("secret request detail"));
    }

    #[test]
    fn successful_response_extracts_trimmed_transcript() {
        let response: MetaTranscribeResponse =
            serde_json::from_str(r#"{"transcript":"  Bonjour  "}"#).unwrap();
        assert_eq!(response.transcript.trim(), "Bonjour");
    }
}
