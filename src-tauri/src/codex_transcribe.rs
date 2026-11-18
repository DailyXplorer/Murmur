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

/// The endpoint publishes no size limit; the public OpenAI transcription API
/// caps uploads at 25 MB (about 13.6 minutes as WAV, the largest form an
/// upload takes), so longer recordings get a warning before upload.
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

fn codex_window_open(spent_at: Option<SystemTime>, now: SystemTime) -> bool {
    within(spent_at, now, CODEX_QUOTA_BACKOFF)
}

/// Whether `now` is less than `window` after `since`. Wall-clock time, so
/// the window also runs out while the Mac sleeps. A clock set back before
/// the window opened ends it.
pub(crate) fn within(since: Option<SystemTime>, now: SystemTime, window: Duration) -> bool {
    since.is_some_and(|since| {
        now.duration_since(since)
            .is_ok_and(|elapsed| elapsed < window)
    })
}

pub fn auth_status() -> CodexAuthStatus {
    codex_auth::auth_status()
}

/// The size of the upload as WAV, which FLAC stays under.
pub fn upload_size_for_samples(sample_count: usize) -> usize {
    44 + sample_count * WAV_BYTES_PER_SAMPLE
}

/// How the audio is encoded for the upload. FLAC is lossless, so the server
/// hears exactly the samples WAV would carry, in about 45 % of the bytes for
/// speech. The endpoint is undocumented and the Codex CLI only ever sent it
/// WAV: FLAC is accepted today, and WAV stays as the form to fall back on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioFormat {
    Flac,
    Wav,
}

impl AudioFormat {
    fn file_name(self) -> &'static str {
        match self {
            Self::Flac => "codex.flac",
            Self::Wav => "codex.wav",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Flac => "audio/flac",
            Self::Wav => "audio/wav",
        }
    }

    fn encode(self, samples: &[f32]) -> Result<Vec<u8>> {
        match self {
            Self::Flac => pcm_f32_to_flac_bytes(samples),
            Self::Wav => pcm_f32_to_wav_bytes(samples),
        }
    }
}

/// A refusal may have been of something else than the format, such as a
/// proxy's challenge page, so FLAC is tried again after this long. If the
/// endpoint did drop FLAC, that costs one refused upload an hour.
const FLAC_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// When the endpoint refused FLAC and took the same audio as WAV. Shared by
/// every request in the process, dictations and parallel meeting uploads.
struct UploadFormat {
    flac_refused_at: Mutex<Option<SystemTime>>,
}

static UPLOAD_FORMAT: UploadFormat = UploadFormat {
    flac_refused_at: Mutex::new(None),
};

impl UploadFormat {
    fn preferred(&self, now: SystemTime) -> AudioFormat {
        if within(*self.refused_at(), now, FLAC_RETRY_AFTER) {
            AudioFormat::Wav
        } else {
            AudioFormat::Flac
        }
    }

    /// The endpoint refused a FLAC upload and took the same audio as WAV.
    /// True when no window was running, so parallel uploads report it once.
    fn flac_refused(&self, now: SystemTime) -> bool {
        let mut refused_at = self.refused_at();
        if within(*refused_at, now, FLAC_RETRY_AFTER) {
            return false;
        }
        *refused_at = Some(now);
        true
    }

    fn refused_at(&self) -> std::sync::MutexGuard<'_, Option<SystemTime>> {
        self.flac_refused_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
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
    let learned = Learned {
        quota: &DICTATION_QUOTA,
        upload_format: &UPLOAD_FORMAT,
    };
    transcribe_with(
        url,
        auth_path,
        samples,
        language,
        uploads_at_once,
        operation,
        learned,
    )
    .await
}

/// What the requests of the process learned about the endpoint so far.
#[derive(Clone, Copy)]
struct Learned<'a> {
    quota: &'a DictationQuota,
    upload_format: &'a UploadFormat,
}

async fn transcribe_with(
    url: &str,
    auth_path: Option<&Path>,
    samples: &[f32],
    language: Option<&str>,
    uploads_at_once: usize,
    operation: &ProcessingOperation,
    learned: Learned<'_>,
) -> Result<String> {
    let Learned {
        quota,
        upload_format,
    } = learned;
    if samples.is_empty() {
        return Ok(String::new());
    }
    if operation.is_cancelled() {
        return Err(anyhow!("Codex transcription cancelled"));
    }

    let mut format = upload_format.preferred(SystemTime::now());
    let mut audio = format.encode(samples)?;
    let upload = Upload::begin();
    let uploads_at_once = uploads_at_once.max(upload.sharing);
    let mut timeout = request_timeout(audio.len(), uploads_at_once);
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
    let mut resent_as_wav = false;
    let mut retries = 0;

    loop {
        if operation.is_cancelled() {
            return Err(anyhow!("Codex transcription cancelled"));
        }
        let request = UploadRequest {
            identity,
            format,
            audio: &audio,
            language,
            timeout,
        };
        let result = transcribe_with_session_at(url, &session, &request, operation).await;
        let error = match result {
            Ok(text) => {
                if resent_as_wav && upload_format.flac_refused(SystemTime::now()) {
                    warn!(
                        "Codex refused FLAC and took WAV; uploads go as WAV for the next {} min",
                        FLAC_RETRY_AFTER.as_secs() / 60
                    );
                }
                return Ok(text);
            }
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

        // A refusal may be of the format: the same audio goes once as WAV,
        // outside the retry budget.
        if error.kind == FailureKind::Rejected && format == AudioFormat::Flac {
            format = AudioFormat::Wav;
            audio = format.encode(samples)?;
            timeout = request_timeout(audio.len(), uploads_at_once);
            resent_as_wav = true;
            continue;
        }

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

/// One upload: who asks, and the audio as it goes out.
struct UploadRequest<'a> {
    identity: RequestIdentity,
    format: AudioFormat,
    audio: &'a [u8],
    language: Option<&'a str>,
    timeout: Duration,
}

async fn transcribe_with_session_at(
    url: &str,
    session: &Session,
    request: &UploadRequest<'_>,
    operation: &ProcessingOperation,
) -> Result<String, RequestError> {
    let UploadRequest {
        identity,
        format,
        audio,
        language,
        timeout,
    } = *request;
    let failed = |kind, detail: String| RequestError::Failed(TranscriptionError::new(kind, detail));
    let client = crate::http::client().map_err(|error| {
        failed(
            FailureKind::Other,
            format!("failed to build transcription HTTP client: {error}"),
        )
    })?;

    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name(format.file_name())
        .mime_str(format.mime())
        .map_err(|error| {
            failed(
                FailureKind::Other,
                format!("invalid audio content type: {error}"),
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
            crate::http::forget();
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

fn to_pcm16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// The samples as 16-bit PCM, converted a block at a time so a ten-minute
/// meeting piece is never held a second time in memory.
struct Pcm16<'a> {
    samples: &'a [f32],
    block: Vec<i32>,
}

impl flacenc::source::Source for Pcm16<'_> {
    fn channels(&self) -> usize {
        1
    }

    fn bits_per_sample(&self) -> usize {
        8 * WAV_BYTES_PER_SAMPLE
    }

    fn sample_rate(&self) -> usize {
        SAMPLE_RATE as usize
    }

    fn read_samples<F: flacenc::source::Fill>(
        &mut self,
        block_size: usize,
        dest: &mut F,
    ) -> Result<usize, flacenc::error::SourceError> {
        let (block, rest) = self.samples.split_at(block_size.min(self.samples.len()));
        self.samples = rest;
        self.block.clear();
        self.block
            .extend(block.iter().map(|sample| i32::from(to_pcm16(*sample))));
        dest.fill_interleaved(&self.block)?;
        Ok(block.len())
    }
}

fn pcm_f32_to_flac_bytes(samples: &[f32]) -> Result<Vec<u8>> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, error)| anyhow!("invalid FLAC encoder settings: {error}"))?;
    let source = Pcm16 {
        samples,
        block: Vec::with_capacity(config.block_size),
    };
    let mut stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|error| anyhow!("failed to encode FLAC: {error}"))?;
    // The encoder records a short last block as the stream's minimum block
    // size. The format excludes the last block from it: a minimum under the
    // maximum declares variable blocks, and a strict decoder then refuses
    // every frame of this fixed-block stream.
    stream
        .stream_info_mut()
        .set_block_sizes(config.block_size, config.block_size)
        .map_err(|error| anyhow!("invalid FLAC block size: {error}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|error| anyhow!("failed to write FLAC: {error}"))?;
    Ok(sink.into_inner())
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
            writer
                .write_sample(to_pcm16(*sample))
                .context("failed to write WAV sample")?;
        }
        writer.finalize().context("failed to finalize WAV")?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod uploaded;
