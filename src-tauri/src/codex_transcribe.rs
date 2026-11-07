//! Transcription through the ChatGPT `transcribe` endpoint with the local
//! Codex session.
//!
//! The same session draws on two dictation quotas, neither documented. A
//! request that names itself as Codex (`originator`, `OAI-Product-Sku`)
//! counts against the Codex dictation quota, which can run out while the
//! account's general Codex usage is low. Without those headers the request
//! counts against ChatGPT's own dictation quota. Requests go as Codex first;
//! when the server answers that the Codex dictation quota is spent, the same
//! audio is resent at once without the Codex headers, and every request in
//! the process skips the Codex identity for `CODEX_QUOTA_BACKOFF`. When
//! ChatGPT's quota is spent during that window, the window closes and the
//! audio is resent as Codex. Each request switches identity at most once.
//!
//! The endpoint takes no vocabulary: for a ChatGPT session, Codex sends only
//! the `file` part, and its `prompt` goes to the API-key endpoint alone
//! (<https://github.com/openai/codex/blob/4e119a3b38e4a4decfccb003acecabc4614142b6/codex-rs/tui/src/voice.rs#L787-L834>).

use anyhow::{anyhow, Context, Result};
use hound::WavSpec;
use log::{info, warn};
use serde::Deserialize;
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

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
    /// The dictation quota of the request's identity is spent.
    DictationQuotaSpent(TranscriptionError),
    Failed(TranscriptionError),
}

/// Which dictation quota a request counts against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestIdentity {
    Codex,
    ChatGpt,
}

impl RequestIdentity {
    fn headers(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Codex => &[
                ("originator", "Codex Desktop"),
                ("OAI-Product-Sku", "CODEX"),
            ],
            Self::ChatGpt => &[],
        }
    }
}

/// The server gives no reset time for the Codex dictation quota. For an hour
/// no dictation first pays the known 429, and a reset is still noticed
/// within the hour.
const CODEX_QUOTA_BACKOFF: Duration = Duration::from_secs(60 * 60);

/// When the Codex dictation quota was found spent. Shared by every request
/// in the process, dictations and parallel meeting uploads.
struct DictationQuota {
    codex_spent_at: Mutex<Option<SystemTime>>,
}

static DICTATION_QUOTA: DictationQuota = DictationQuota {
    codex_spent_at: Mutex::new(None),
};

impl DictationQuota {
    fn identity(&self, now: SystemTime) -> RequestIdentity {
        if codex_window_open(*self.spent_at(), now) {
            RequestIdentity::ChatGpt
        } else {
            RequestIdentity::Codex
        }
    }

    /// Remembers the spent Codex quota. True when no window was running, so
    /// the parallel requests that all hit the quota report it once.
    fn codex_spent(&self, now: SystemTime) -> bool {
        let mut spent_at = self.spent_at();
        if codex_window_open(*spent_at, now) {
            return false;
        }
        *spent_at = Some(now);
        true
    }

    fn codex_available(&self) {
        *self.spent_at() = None;
    }

    fn spent_at(&self) -> std::sync::MutexGuard<'_, Option<SystemTime>> {
        self.codex_spent_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Wall-clock time, so the window also runs out while the Mac sleeps. A
/// clock set back before the window opened ends it.
fn codex_window_open(spent_at: Option<SystemTime>, now: SystemTime) -> bool {
    spent_at.is_some_and(|spent_at| {
        now.duration_since(spent_at)
            .is_ok_and(|elapsed| elapsed < CODEX_QUOTA_BACKOFF)
    })
}

pub fn auth_status() -> CodexAuthStatus {
    codex_auth::auth_status()
}

pub fn upload_size_for_samples(sample_count: usize) -> usize {
    44 + sample_count * WAV_BYTES_PER_SAMPLE
}

/// Uploads to Codex under way, from dictations and meeting pieces alike:
/// they share the uplink.
static UPLOADS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// One upload under way, until dropped.
struct Upload {
    /// Uploads under way as this one started, itself included.
    sharing: usize,
}

impl Upload {
    fn begin() -> Self {
        Self {
            sharing: UPLOADS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1,
        }
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        UPLOADS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
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
/// default when `None`), while `uploads_at_once` requests share the uplink,
/// or more if more uploads are already under way.
pub(crate) async fn transcribe_at(
    url: &str,
    auth_path: Option<&Path>,
    samples: &[f32],
    language: Option<&str>,
    uploads_at_once: usize,
    operation: &ProcessingOperation,
) -> Result<String> {
    transcribe_with_quota(
        url,
        auth_path,
        samples,
        language,
        uploads_at_once,
        operation,
        &DICTATION_QUOTA,
    )
    .await
}

async fn transcribe_with_quota(
    url: &str,
    auth_path: Option<&Path>,
    samples: &[f32],
    language: Option<&str>,
    uploads_at_once: usize,
    operation: &ProcessingOperation,
    quota: &DictationQuota,
) -> Result<String> {
    if samples.is_empty() {
        return Ok(String::new());
    }
    if operation.is_cancelled() {
        return Err(anyhow!("Codex transcription cancelled"));
    }

    let wav = pcm_f32_to_wav_bytes(samples)?;
    let upload = Upload::begin();
    let timeout = request_timeout(wav.len(), uploads_at_once.max(upload.sharing));
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
    let mut identity = quota.identity(SystemTime::now());
    let mut switched_identity = false;
    let mut recovered_from_unauthorized = false;
    let mut retries = 0;

    loop {
        if operation.is_cancelled() {
            return Err(anyhow!("Codex transcription cancelled"));
        }
        let result =
            transcribe_with_session_at(url, &session, identity, &wav, language, timeout, operation)
                .await;
        let error = match result {
            Ok(text) => return Ok(text),
            Err(RequestError::Cancelled) => return Err(anyhow!("Codex transcription cancelled")),
            Err(RequestError::DictationQuotaSpent(error)) => {
                if identity == RequestIdentity::Codex && quota.codex_spent(SystemTime::now()) {
                    info!(
                        "Codex dictation quota reached; using ChatGPT dictation quota for the next {} min",
                        CODEX_QUOTA_BACKOFF.as_secs() / 60
                    );
                }
                if switched_identity {
                    error
                } else {
                    // The other quota is a separate count, so resend at once
                    // and outside the retry budget.
                    switched_identity = true;
                    identity = match identity {
                        RequestIdentity::Codex => RequestIdentity::ChatGpt,
                        RequestIdentity::ChatGpt => {
                            info!(
                                "ChatGPT dictation quota reached; trying the Codex dictation quota again"
                            );
                            quota.codex_available();
                            RequestIdentity::Codex
                        }
                    };
                    continue;
                }
            }
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
    identity: RequestIdentity,
    wav: &[u8],
    language: Option<&str>,
    timeout: Duration,
    operation: &ProcessingOperation,
) -> Result<String, RequestError> {
    let failed = |kind, detail: String| RequestError::Failed(TranscriptionError::new(kind, detail));
    let client = crate::http::client(CONNECT_TIMEOUT).map_err(|error| {
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
        .timeout(timeout)
        .header("Authorization", format!("Bearer {}", session.access_token))
        .multipart(form);

    for (name, value) in identity.headers() {
        request = request.header(*name, *value);
    }
    if let Some(account_id) = session.account_id.as_deref() {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = tokio::select! {
        _ = operation.cancelled() => return Err(RequestError::Cancelled),
        response = request.send() => response.map_err(|error| {
            // Any transport failure may mean a dead pooled connection or a
            // changed network or proxy: the next request starts fresh.
            crate::http::forget_all();
            failed(
                FailureKind::from_transport(&error),
                format!("Codex transcription request failed: {error}"),
            )
        })?,
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
        let error = TranscriptionError::new(
            kind,
            format!("Codex transcription failed with HTTP {status}: {excerpt}"),
        );
        return Err(if is_dictation_quota_spent(status.as_u16(), &body) {
            RequestError::DictationQuotaSpent(error)
        } else {
            RequestError::Failed(error)
        });
    }

    parse_transcript(&body).map_err(|error| failed(FailureKind::Other, error.to_string()))
}

/// A spent dictation quota answers 429 with
/// `{"detail":"Codex dictation usage limit reached"}`; other 429s are
/// short-term rate limits.
fn is_dictation_quota_spent(status: u16, body: &str) -> bool {
    status == 429
        && serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .as_ref()
            .and_then(|json| json.get("detail")?.as_str())
            .is_some_and(|detail| {
                detail
                    .to_ascii_lowercase()
                    .contains("dictation usage limit")
            })
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
mod tests;
