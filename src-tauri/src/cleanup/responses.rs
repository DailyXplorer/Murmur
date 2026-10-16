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
        "reasoning": { "effort": "low" },
        "store": false,
        "include": [],
        "text": { "verbosity": "low" },
    });

    let mut builder = client
        .post(url)
        .header("Authorization", format!("Bearer {}", session.access_token))
        .header("originator", "Codex Desktop")
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
            let Ok(event) = serde_json::from_str::<StreamEvent>(data.trim()) else {
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
                    return Err(CompletionError::failed(
                        FailureKind::ServerError,
                        format!("cleanup stream reported {}", event.kind),
                    ));
                }
                _ => {}
            }
        }
        Ok(None)
    }
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
    let cache: ModelsCache = serde_json::from_str(models_cache_json).ok()?;
    cache
        .models
        .into_iter()
        .filter(|model| model.visibility.as_deref() != Some("hide"))
        .filter(|model| {
            let slug = model.slug.to_ascii_lowercase();
            FAST_MODEL_MARKERS
                .iter()
                .any(|marker| slug.contains(marker))
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
    fn a_missing_cache_falls_back_to_the_known_fast_model() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(fast_model(directory.path()), FALLBACK_MODEL);
    }
}
