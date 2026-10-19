//! One tool-less request to the ChatGPT Codex responses endpoint, made the way
//! the Codex CLI makes its own one-shot classifier calls: the signed-in
//! session's bearer token, a developer message plus a user message, no
//! tools, `store: false`, and a streamed reply.

use crate::codex_auth::Session;
use crate::transcription_error::FailureKind;
use crate::ProcessingOperation;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::time::Duration;

pub const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// What the Codex CLI itself uses for fast one-shot classification.
const FALLBACK_MODEL: &str = "gpt-6-luna";
/// Slug fragments of the small, fast tier across model generations.
const FAST_MODEL_MARKERS: [&str; 4] = ["luna", "mini", "nano", "flash"];

#[derive(Debug)]
pub enum CompletionError {
    Cancelled,
    /// The reply grew past the cap: the model is not editing, it is writing.
    TooLong,
    Failed {
        kind: FailureKind,
        detail: String,
    },
    /// The on-device model gave no reply.
    Apple(super::apple::AppleError),
}

impl From<super::apple::AppleError> for CompletionError {
    fn from(error: super::apple::AppleError) -> Self {
        Self::Apple(error)
    }
}

impl From<crate::transcription_error::TranscriptionError> for CompletionError {
    fn from(error: crate::transcription_error::TranscriptionError) -> Self {
        Self::Failed {
            kind: error.kind,
            detail: error.detail,
        }
    }
}

impl CompletionError {
    /// The server does not serve this model to this account, as it answers
    /// for a model outside the ChatGPT plan: HTTP 400, "The 'x' model is not
    /// supported when using Codex with a ChatGPT account."
    pub fn is_model_unsupported(&self) -> bool {
        matches!(
            self,
            Self::Failed { kind: FailureKind::Rejected, detail }
                if detail.contains("HTTP 400") && detail.contains("model is not supported")
        )
    }

    fn failed(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self::Failed {
            kind,
            detail: detail.into(),
        }
    }
}

pub struct Request<'a> {
    pub model: &'a str,
    pub instructions: &'a str,
    pub user_message: &'a str,
    pub max_output_chars: usize,
    /// `low`, `medium` or `high`: how long the model reasons first.
    pub effort: &'a str,
    /// `low`, `medium` or `high`: how much the model writes.
    pub verbosity: &'a str,
}

/// Posts one request and returns the streamed output text. The caller owns
/// the overall deadline; this only bounds connecting.
pub async fn complete(
    url: &str,
    session: &Session,
    request: &Request<'_>,
    operation: &ProcessingOperation,
) -> Result<String, CompletionError> {
    let client = reqwest::Client::builder()
        .user_agent(crate::codex_transcribe::user_agent())
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| CompletionError::failed(FailureKind::Other, error.to_string()))?;

    let body = json!({
        "model": request.model,
        "stream": true,
        "input": [
            message("developer", request.instructions),
            message("user", request.user_message),
        ],
        "tool_choice": "none",
        "parallel_tool_calls": false,
        "reasoning": { "effort": request.effort },
        "store": false,
        "include": [],
        "text": { "verbosity": request.verbosity },
    });

    let mut builder = client
        .post(url)
        .header("Authorization", format!("Bearer {}", session.access_token))
        .header("originator", crate::codex_transcribe::originator())
        .header("Accept", "text/event-stream")
        .json(&body);
    if let Some(account_id) = session.account_id.as_deref() {
        builder = builder.header("ChatGPT-Account-Id", account_id);
    }

    let mut response = tokio::select! {
        _ = operation.cancelled() => return Err(CompletionError::Cancelled),
        response = builder.send() => response.map_err(|error| CompletionError::failed(
            FailureKind::from_transport(&error),
            format!("cleanup request failed: {error}"),
        ))?,
    };

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let excerpt: String = body.chars().take(200).collect();
        return Err(CompletionError::failed(
            FailureKind::from_status(status.as_u16()),
            format!("cleanup failed with HTTP {status}: {excerpt}"),
        ));
    }

    let mut stream = SseText::new(request.max_output_chars);
    loop {
        let chunk = tokio::select! {
            _ = operation.cancelled() => return Err(CompletionError::Cancelled),
            chunk = response.chunk() => chunk.map_err(|error| CompletionError::failed(
                FailureKind::from_transport(&error),
                format!("cleanup stream failed: {error}"),
            ))?,
        };
        let Some(chunk) = chunk else {
            return Err(CompletionError::failed(
                FailureKind::Network,
                "cleanup stream ended before completion",
            ));
        };
        if let Some(text) = stream.push(&chunk)? {
            return Ok(text);
        }
    }
}

fn message(role: &str, text: &str) -> serde_json::Value {
    json!({
        "type": "message",
        "role": role,
        "content": [{ "type": "input_text", "text": text }],
    })
}

#[derive(Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<String>,
}

/// Incremental parser for the server-sent events of a responses stream.
struct SseText {
    pending: Vec<u8>,
    text: String,
    max_chars: usize,
}

impl SseText {
    fn new(max_chars: usize) -> Self {
        Self {
            pending: Vec::new(),
            text: String::new(),
            max_chars,
        }
    }

    /// Feeds raw bytes. Returns the whole output once the response completes.
    fn push(&mut self, bytes: &[u8]) -> Result<Option<String>, CompletionError> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim_end().strip_prefix("data:") else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(data.trim()) else {
                continue;
            };
            let Ok(event) = StreamEvent::deserialize(&json) else {
                continue;
            };
            match event.kind.as_str() {
                "response.output_text.delta" => {
                    self.text
                        .push_str(event.delta.as_deref().unwrap_or_default());
                    if self.text.chars().count() > self.max_chars {
                        return Err(CompletionError::TooLong);
                    }
                }
                "response.completed" => return Ok(Some(std::mem::take(&mut self.text))),
                "response.failed" | "response.incomplete" | "error" => {
                    return Err(stream_failure(&event.kind, &json));
                }
                _ => {}
            }
        }
        Ok(None)
    }
}

/// Classes a failed stream the way an HTTP answer would be, from the codes
/// the Codex CLI reads (`codex-api/src/sse/responses.rs`).
fn stream_failure(event: &str, json: &serde_json::Value) -> CompletionError {
    let response = json.get("response");
    let reason = response
        .and_then(|response| response.pointer("/incomplete_details/reason"))
        .and_then(serde_json::Value::as_str);
    let code = response
        .and_then(|response| response.get("error"))
        .or_else(|| json.get("error"))
        .unwrap_or(json)
        .get("code")
        .and_then(serde_json::Value::as_str);
    let kind = match (reason, code) {
        (Some("max_output_tokens"), _) => return CompletionError::TooLong,
        (Some("content_filter"), _) => FailureKind::Rejected,
        (_, Some("rate_limit_exceeded" | "slow_down" | "insufficient_quota")) => {
            FailureKind::RateLimited
        }
        (_, Some("context_length_exceeded")) => FailureKind::TooLarge,
        _ => FailureKind::ServerError,
    };
    let what = reason.or(code).unwrap_or("no code");
    CompletionError::failed(kind, format!("cleanup stream reported {event} ({what})"))
}

#[derive(Deserialize)]
struct ModelsCache {
    models: Vec<CachedModel>,
}

#[derive(Deserialize)]
struct CachedModel {
    slug: String,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
}

/// The fast model the account can use, read from the model list the Codex
/// CLI caches next to `auth.json`, so a renamed tier is picked up without a
/// Murmur release.
pub fn fast_model(codex_home: &Path) -> String {
    std::fs::read_to_string(codex_home.join("models_cache.json"))
        .ok()
        .and_then(|json| pick_fast_model(&json))
        .unwrap_or_else(|| FALLBACK_MODEL.to_string())
}

fn pick_fast_model(models_cache_json: &str) -> Option<String> {
    pick_model(models_cache_json, true)
}

/// The standard (not fast) model the account can use, for work where
/// quality matters more than speed; the fast model when none is listed.
///
/// The list comes from the server when it answers, since it depends on the
/// account's plan: the Codex CLI's cache keeps the list of the plan it was
/// fetched under, so after a downgrade it names models the server refuses.
pub async fn default_model(models_url: &str, session: &Session, codex_home: &Path) -> String {
    let cached = std::fs::read_to_string(codex_home.join("models_cache.json")).ok();
    let listed = match live_models(models_url, session, cached.as_deref()).await {
        Ok(json) => Some(json),
        Err(error) => {
            log::warn!("Codex model list unavailable, using the cached one: {error}");
            cached
        }
    };
    listed
        .and_then(|json| pick_model(&json, false))
        .unwrap_or_else(|| fast_model(codex_home))
}

/// Where the account's model list is served, next to the responses endpoint.
pub fn models_url(responses_url: &str) -> String {
    format!("{}/models", responses_url.trim_end_matches("/responses"))
}

/// The server requires the Codex CLI version it filters the list by; the one
/// that wrote the cache, or the one Murmur was last checked against.
async fn live_models(url: &str, session: &Session, cached: Option<&str>) -> Result<String, String> {
    let client_version = cached
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|json| json["client_version"].as_str().map(str::to_string))
        .unwrap_or_else(|| crate::codex_transcribe::CODEX_CLI_VERSION.to_string());
    let client = reqwest::Client::builder()
        .user_agent(crate::codex_transcribe::user_agent())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    let mut builder = client
        .get(url)
        .query(&[("client_version", client_version)])
        .header("Authorization", format!("Bearer {}", session.access_token))
        .header("originator", crate::codex_transcribe::originator());
    if let Some(account_id) = session.account_id.as_deref() {
        builder = builder.header("ChatGPT-Account-Id", account_id);
    }
    let response = builder.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let json = response.text().await.map_err(|error| error.to_string())?;
    serde_json::from_str::<ModelsCache>(&json).map_err(|error| error.to_string())?;
    Ok(json)
}

/// Models the server refused for this account since launch.
static REFUSED_MODELS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Keeps `model` out of every later choice until Murmur quits.
pub fn forget_model(model: &str) {
    if let Ok(mut refused) = REFUSED_MODELS.lock() {
        refused.push(model.to_string());
    }
}

fn is_refused(model: &str) -> bool {
    REFUSED_MODELS
        .lock()
        .is_ok_and(|refused| refused.iter().any(|slug| slug == model))
}

/// The listed model of the first rank in the fast or the standard tier.
fn pick_model(models_cache_json: &str, fast: bool) -> Option<String> {
    let cache: ModelsCache = serde_json::from_str(models_cache_json).ok()?;
    cache
        .models
        .into_iter()
        .filter(|model| model.visibility.as_deref() != Some("hide"))
        .filter(|model| !is_refused(&model.slug))
        .filter(|model| {
            let slug = model.slug.to_ascii_lowercase();
            FAST_MODEL_MARKERS
                .iter()
                .any(|marker| slug.contains(marker))
                == fast
        })
        .min_by_key(|model| model.priority.unwrap_or(i64::MAX))
        .map(|model| model.slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(parser: &mut SseText, events: &str) -> Result<Option<String>, CompletionError> {
        parser.push(events.as_bytes())
    }

    #[test]
    fn deltas_accumulate_until_completion_even_across_chunk_splits() {
        let mut parser = SseText::new(100);
        let stream = "event: response.created\ndata: {\"type\":\"response.created\"}\n\n\
event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"So the meeting \"}\n\n\
event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"is at 3 pm.\"}\n\n\
event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n";
        let (first, second) = stream.split_at(97);
        assert!(feed(&mut parser, first).unwrap().is_none());
        assert_eq!(
            feed(&mut parser, second).unwrap().as_deref(),
            Some("So the meeting is at 3 pm.")
        );
    }

    #[test]
    fn runaway_output_is_cut_off() {
        let mut parser = SseText::new(10);
        let result = feed(
            &mut parser,
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"far more than ten characters\"}\n",
        );
        assert!(matches!(result, Err(CompletionError::TooLong)));
    }

    #[test]
    fn a_failed_response_is_an_error() {
        let mut parser = SseText::new(100);
        let result = feed(
            &mut parser,
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{}}}\n",
        );
        assert!(matches!(
            result,
            Err(CompletionError::Failed {
                kind: FailureKind::ServerError,
                ..
            })
        ));
    }

    fn failure_kind(event: &str) -> Option<FailureKind> {
        match feed(&mut SseText::new(100), &format!("data: {event}\n")) {
            Err(CompletionError::Failed { kind, .. }) => Some(kind),
            Err(CompletionError::TooLong) => None,
            _ => panic!("{event} must fail"),
        }
    }

    #[test]
    fn stream_failures_are_classified_like_http_answers() {
        for code in ["rate_limit_exceeded", "slow_down", "insufficient_quota"] {
            assert_eq!(
                failure_kind(&format!(
                    r#"{{"type":"response.failed","response":{{"error":{{"code":"{code}","message":"x"}}}}}}"#
                )),
                Some(FailureKind::RateLimited),
                "{code}"
            );
        }
        assert_eq!(
            failure_kind(
                r#"{"type":"response.failed","response":{"error":{"code":"context_length_exceeded"}}}"#
            ),
            Some(FailureKind::TooLarge)
        );
        assert_eq!(
            failure_kind(r#"{"type":"error","code":"rate_limit_exceeded","message":"x"}"#),
            Some(FailureKind::RateLimited)
        );
        assert_eq!(
            failure_kind(r#"{"type":"error","error":{"code":"insufficient_quota"}}"#),
            Some(FailureKind::RateLimited)
        );
        assert_eq!(
            failure_kind(
                r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}"#
            ),
            None
        );
        assert_eq!(
            failure_kind(
                r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}"#
            ),
            Some(FailureKind::Rejected)
        );
        assert_eq!(
            failure_kind(
                r#"{"type":"response.failed","response":{"error":{"code":"server_is_overloaded"}}}"#
            ),
            Some(FailureKind::ServerError)
        );
    }

    #[test]
    fn the_newest_listed_fast_model_is_chosen() {
        let cache = r#"{"models":[
            {"slug":"gpt-6.1-sol","visibility":"list","priority":1},
            {"slug":"gpt-6-luna","visibility":"list","priority":4},
            {"slug":"gpt-5.6-luna","visibility":"list","priority":9},
            {"slug":"secret-mini","visibility":"hide","priority":0}
        ]}"#;
        assert_eq!(pick_fast_model(cache).as_deref(), Some("gpt-6-luna"));
        assert_eq!(
            pick_fast_model(r#"{"models":[{"slug":"gpt-6.1-sol"}]}"#),
            None
        );
        assert_eq!(pick_fast_model("not json"), None);
    }

    #[test]
    fn the_first_listed_standard_model_is_the_default() {
        let cache = r#"{"models":[
            {"slug":"gpt-6-luna","visibility":"list","priority":0},
            {"slug":"gpt-6.1-sol","visibility":"list","priority":3},
            {"slug":"gpt-6.2-sol","visibility":"list","priority":1},
            {"slug":"secret-max","visibility":"hide","priority":0}
        ]}"#;
        assert_eq!(pick_model(cache, false).as_deref(), Some("gpt-6.2-sol"));
        assert_eq!(
            pick_model(r#"{"models":[{"slug":"gpt-6-luna"}]}"#, false),
            None
        );
    }

    #[test]
    fn a_refused_model_is_never_picked_again() {
        let cache = r#"{"models":[
            {"slug":"gpt-test-refused-sol","priority":1},
            {"slug":"gpt-test-kept-terra","priority":2}
        ]}"#;
        assert_eq!(
            pick_model(cache, false).as_deref(),
            Some("gpt-test-refused-sol")
        );
        forget_model("gpt-test-refused-sol");
        assert_eq!(
            pick_model(cache, false).as_deref(),
            Some("gpt-test-kept-terra")
        );
    }

    #[test]
    fn only_the_plan_refusal_counts_as_an_unsupported_model() {
        let refusal = CompletionError::failed(
            FailureKind::Rejected,
            r#"cleanup failed with HTTP 400 Bad Request: {"detail":"The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account."}"#,
        );
        assert!(refusal.is_model_unsupported());
        let other = CompletionError::failed(
            FailureKind::Rejected,
            "cleanup failed with HTTP 400 Bad Request: {}",
        );
        assert!(!other.is_model_unsupported());
    }

    #[test]
    fn the_models_endpoint_sits_next_to_the_responses_one() {
        assert_eq!(
            models_url(RESPONSES_URL),
            "https://chatgpt.com/backend-api/codex/models"
        );
    }

    #[test]
    fn a_missing_cache_falls_back_to_the_known_fast_model() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(fast_model(directory.path()), FALLBACK_MODEL);
    }
}
