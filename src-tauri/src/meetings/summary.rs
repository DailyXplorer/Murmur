//! The meeting's title and summary, written by Codex from the transcript in
//! one responses call (or one per hour of talk, then one to merge them, for
//! a very long meeting). People appear in the summary only as `[[key]]`
//! markers, so renaming a speaker changes the summary without rewriting it.
//! The title is plain text about the topic and names no one.

use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use super::export::clock;
use super::merge::PLACEHOLDER;
use super::rename::MAX_TITLE_CHARS;
use super::store::{MeetingSpeaker, MeetingTurn, MeetingsStore, SummaryStatus};
use super::transcribe::{unreachable, CodexVerdict, RetryPolicy};
use crate::chinese_script::in_chosen_script;
use crate::cleanup::responses::{self, CompletionError, Request};
use crate::codex_auth;
use crate::transcription_error::FailureKind;
use crate::ProcessingOperation;

/// About three hours of talk fit in one request; past this, the transcript
/// is summarised in sections first.
pub const MAX_SUMMARY_CHARS: usize = 400_000;
/// About an hour of talk.
const SECTION_CHARS: usize = 150_000;
const MAX_OUTPUT_CHARS: usize = 24_000;
/// Per request: a summary takes Codex well under a minute.
const DEADLINE: Duration = Duration::from_secs(180);

const SHAPE: &str = "Write in the language most of the transcript is spoken in.

Output Markdown only, in exactly this shape:
# <title of at most 8 words>
<one context paragraph of at most 3 sentences: what the meeting was about and who took part>
**<Section heading>**
- <point>
  - <detail; at most one nested level>
Then one such section per topic, in the order the topics came up. Then, only when the meeting had them:
**Decisions**
- <decision>
**Action items**
- <owner> — <task> — <due date, only when one was stated>
Translate the headings \"Decisions\" and \"Action items\" into the language of the meeting.

Refer to the people of the transcript only by their marker, exactly as written there, such as [[me]] or [[s2]]; never write their names. When a speaker says \"I\" or \"I will\", the owner is that speaker's marker. The title is about the topic only: it names no one, by marker or otherwise.
Leave a blank line between blocks. No preamble, no code fences, no closing remarks. Do not invent anything that was not said.";

const TRANSCRIPT_FORMAT: &str = "Each line of the transcript is one passage: \"[time] [[key]] Name: text\", where [[key]] is the speaker's marker.";

fn instructions() -> String {
    format!(
        "You write the notes of a meeting from its transcript.\n\n{TRANSCRIPT_FORMAT}\n\n{SHAPE}"
    )
}

fn section_instructions() -> String {
    format!(
        "You write the notes of one part of a long meeting from its transcript. Other parts are summarised separately and merged later.\n\n{TRANSCRIPT_FORMAT}\n\n{SHAPE}"
    )
}

/// Most terms of the user's vocabulary a summary request names. The words
/// are at most 50 characters each, so the line stays under 1.5 kB however
/// long the dictionary grows.
const MAX_VOCABULARY_TERMS: usize = 30;

/// What the summary is told about the user besides the transcript.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hints<'a> {
    /// The dictation language; Chinese is written in the script it chose.
    pub language: Option<&'a str>,
    /// The user's custom words, so a name the transcript spelled wrongly is
    /// written as they spell it.
    pub vocabulary: &'a [String],
}

/// "Names and terms that may appear: …", or `None` without a usable term.
/// Terms are trimmed, deduplicated and cut at [`MAX_VOCABULARY_TERMS`], in
/// the order the user keeps them.
fn vocabulary_line(vocabulary: &[String]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let terms: Vec<String> = vocabulary
        .iter()
        .map(|term| term.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|term| !term.is_empty() && seen.insert(term.clone()))
        .take(MAX_VOCABULARY_TERMS)
        .collect();
    (!terms.is_empty()).then(|| {
        format!(
            "Names and terms that may appear: {}. Spell them this way, in the title as well, when the transcript refers to them.",
            terms.join(", ")
        )
    })
}

fn with_vocabulary(instructions: String, vocabulary: &[String]) -> String {
    match vocabulary_line(vocabulary) {
        Some(line) => format!("{instructions}\n\n{line}"),
        None => instructions,
    }
}

fn merge_instructions() -> String {
    format!(
        "You merge the notes of the consecutive parts of one long meeting into the notes of the whole meeting: one title, one context paragraph, sections that combine the same topic across parts, and the decisions and action items of all parts without repeats.\n\n{SHAPE}"
    )
}

/// What went wrong writing a summary, as the user is told.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SummaryError {
    /// Codex needs a new sign-in.
    SignedOut,
    RateLimited,
    /// Codex could not be reached.
    Network,
    /// Codex answered with a server error, or its reply was cut off.
    Server,
    Timeout,
    /// The reply ran past its length cap.
    TooLong,
    /// The reply had no summary in it.
    Empty,
    /// No words were transcribed: there is nothing to summarise.
    NothingToSummarize,
    Other,
}

impl SummaryError {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::SignedOut => "signed_out",
            Self::RateLimited => "rate_limited",
            Self::Network => "network",
            Self::Server => "server",
            Self::Timeout => "timeout",
            Self::TooLong => "too_long",
            Self::Empty => "empty",
            Self::NothingToSummarize => "nothing_to_summarize",
            Self::Other => "other",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "signed_out" => Self::SignedOut,
            "rate_limited" => Self::RateLimited,
            "network" => Self::Network,
            "server" => Self::Server,
            "timeout" => Self::Timeout,
            "too_long" => Self::TooLong,
            "empty" => Self::Empty,
            "nothing_to_summarize" => Self::NothingToSummarize,
            "other" => Self::Other,
            _ => return None,
        })
    }

    fn of(kind: FailureKind) -> Self {
        match kind {
            FailureKind::SessionExpired | FailureKind::SignedOut => Self::SignedOut,
            FailureKind::RateLimited => Self::RateLimited,
            FailureKind::Network => Self::Network,
            FailureKind::ServerError | FailureKind::ConnectionDropped => Self::Server,
            FailureKind::Timeout => Self::Timeout,
            FailureKind::TooLarge
            | FailureKind::Rejected
            | FailureKind::LocalNotReady
            | FailureKind::LocalFailed
            | FailureKind::LanguageNeedsCloud
            | FailureKind::Other => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// `None` when the reply had no title line.
    pub title: Option<String>,
    /// Markdown, with `[[key]]` markers for people.
    pub body: String,
}

/// One line per bubble, `[04:07] [[s2]] Speaker 2: text`, with the names as
/// they are now. Parts that could not be transcribed are left out.
pub fn transcript_lines(turns: &[MeetingTurn], speakers: &[MeetingSpeaker]) -> Vec<String> {
    let names: HashMap<&str, &str> = speakers
        .iter()
        .map(|speaker| (speaker.key.as_str(), speaker.name.as_str()))
        .collect();
    turns
        .iter()
        .filter(|turn| turn.speaker_key != PLACEHOLDER && !turn.text.trim().is_empty())
        .map(|turn| {
            let key = turn.speaker_key.as_str();
            format!(
                "[{}] [[{key}]] {}: {}",
                clock(turn.start_ms),
                names.get(key).copied().unwrap_or(key),
                turn.text.trim()
            )
        })
        .collect()
}

/// The lines in consecutive sections of at most `max_chars` (a single longer
/// line stays whole).
fn sections(lines: &[String], max_chars: usize) -> Vec<String> {
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in lines {
        let chars = line.chars().count() + 1;
        if !current.is_empty() && current.chars().count() + chars > max_chars {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
}

/// Reads the reply: the first line `# Title` is the title, the rest the
/// body, with code fences removed and lists at most two levels deep. The
/// title is plain text that a rename does not reach, so a marker Codex put
/// in it despite the rules becomes the speaker's name as it is now.
pub fn parse(output: &str, speakers: &[MeetingSpeaker]) -> Result<Summary, SummaryError> {
    let mut lines: Vec<&str> = output.trim().lines().collect();
    if lines
        .first()
        .is_some_and(|line| line.trim_start().starts_with("```"))
    {
        lines.remove(0);
        if lines.last().is_some_and(|line| line.trim() == "```") {
            lines.pop();
        }
    }
    let first = lines.iter().position(|line| !line.trim().is_empty());
    let mut title = None;
    if let Some(first) = first {
        if let Some(heading) = lines[first].trim().strip_prefix("# ") {
            let heading = heading.trim().trim_matches('*').trim();
            if !heading.is_empty() {
                let names = speakers
                    .iter()
                    .map(|speaker| (speaker.key.as_str(), speaker.name.clone()))
                    .collect();
                title = Some(named_title(heading, &names));
            }
            lines.remove(first);
        }
    }
    let body: Vec<String> = lines.iter().map(|line| flatten_list(line)).collect();
    let body = body.join("\n").trim().to_string();
    if body.is_empty() {
        return Err(SummaryError::Empty);
    }
    Ok(Summary { title, body })
}

/// `- point` at the top level, `  - detail` below it; deeper levels join
/// the second, and `*` or `+` bullets become `-`.
fn flatten_list(line: &str) -> String {
    let content = line.trim_start();
    let indent = line.len() - content.len();
    let item = ["- ", "* ", "+ "]
        .iter()
        .find_map(|bullet| content.strip_prefix(bullet));
    // `**Heading**` starts with `*` but is no list item.
    match item {
        Some(text) if !content.starts_with("**") => {
            let prefix = if indent >= 2 { "  - " } else { "- " };
            format!("{prefix}{}", text.trim())
        }
        _ => line.trim_end().to_string(),
    }
}

/// A title with its markers replaced by names, cut to the title limit.
pub fn named_title(title: &str, names: &HashMap<&str, String>) -> String {
    resolve_markers(title, names)
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// Replaces each `[[key]]` with the speaker's name, or with the bare key
/// when no speaker has it.
pub fn resolve_markers(text: &str, names: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let key = &after[..end];
        if key.is_empty() || key.chars().any(|c| c.is_whitespace() || c == '[') {
            out.push_str(&rest[..start + 2]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(names.get(key).map(String::as_str).unwrap_or(key));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

pub type CompleteFuture = Pin<Box<dyn Future<Output = Result<String, CompletionError>> + Send>>;

/// A model that answers one request; Codex in the app, a script in tests.
pub trait Summarizer: Send + Sync + 'static {
    fn complete(
        &self,
        instructions: String,
        input: String,
        operation: ProcessingOperation,
    ) -> CompleteFuture;
}

/// The Codex responses endpoint with the standard model, through the
/// session Codex keeps on disk.
pub struct CodexSummarizer {
    pub url: String,
    /// The Codex default when `None`.
    pub auth_path: Option<PathBuf>,
}

impl Summarizer for CodexSummarizer {
    fn complete(
        &self,
        instructions: String,
        input: String,
        operation: ProcessingOperation,
    ) -> CompleteFuture {
        let url = self.url.clone();
        let auth_path = self.auth_path.clone();
        Box::pin(async move {
            let auth_path = match auth_path {
                Some(path) => path,
                None => codex_auth::auth_path()?,
            };
            let codex_home = auth_path.parent().unwrap_or(&auth_path);
            let mut session = codex_auth::session_for_request(&auth_path).await?;
            let model =
                responses::default_model(&responses::models_url(&url), &session, codex_home).await;
            let mut request = summary_request(&model, &instructions, &input);
            let mut result = responses::complete(&url, &session, &request, &operation).await;
            if let Err(CompletionError::Failed {
                kind: FailureKind::SessionExpired,
                ..
            }) = result
            {
                session = codex_auth::recover_from_unauthorized(&auth_path, &session).await?;
                result = responses::complete(&url, &session, &request, &operation).await;
            }
            let fast = responses::fast_model(codex_home);
            match result {
                Err(error) if error.is_model_unsupported() && model != fast => {
                    log::warn!("Codex refused {model} for this account; summarising with {fast}");
                    responses::forget_model(&model);
                    request.model = &fast;
                    responses::complete(&url, &session, &request, &operation).await
                }
                other => other,
            }
        })
    }
}

/// One summary call to Codex.
fn summary_request<'a>(model: &'a str, instructions: &'a str, input: &'a str) -> Request<'a> {
    Request {
        model,
        instructions,
        user_message: input,
        max_output_chars: MAX_OUTPUT_CHARS,
        effort: "medium",
        verbosity: "medium",
        // As long as a transcription piece: the summary has no fallback, and
        // retrying on the same slow link would only time out again.
        connect_timeout: crate::codex_transcribe::CONNECT_TIMEOUT,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SummaryEnd {
    Written(Summary),
    Failed(SummaryError),
    /// Codex could not be reached once the retries were spent.
    Unreachable,
    Cancelled,
}

/// One request, retried like a transcription piece: rate limits back off
/// within the policy's budget, and network, timeout and server failures
/// retry for a few minutes. Each attempt has its own deadline.
async fn ask(
    summarizer: &dyn Summarizer,
    policy: &RetryPolicy,
    instructions: String,
    input: String,
    operation: &ProcessingOperation,
) -> Result<String, SummaryEnd> {
    let mut transient = 0;
    let mut rate_limits = 0u32;
    let mut waited = Duration::ZERO;
    loop {
        let kind = match tokio::time::timeout(
            DEADLINE,
            summarizer.complete(instructions.clone(), input.clone(), operation.clone()),
        )
        .await
        {
            Ok(Ok(output)) => return Ok(output),
            Ok(Err(CompletionError::Cancelled)) => return Err(SummaryEnd::Cancelled),
            Ok(Err(CompletionError::TooLong)) => {
                return Err(SummaryEnd::Failed(SummaryError::TooLong))
            }
            Ok(Err(CompletionError::Failed { kind, detail })) => {
                log::warn!("Meeting summary request failed ({kind:?}): {detail}");
                kind
            }
            Err(_) => FailureKind::Timeout,
        };
        let delay = match kind {
            FailureKind::RateLimited => {
                let delay = policy.rate_limit_delay(rate_limits);
                rate_limits += 1;
                waited += delay;
                (waited <= policy.rate_limit_budget).then_some(delay)
            }
            FailureKind::Network
            | FailureKind::Timeout
            | FailureKind::ServerError
            | FailureKind::ConnectionDropped => {
                transient += 1;
                policy.transient.get(transient - 1).copied()
            }
            _ => None,
        };
        let Some(delay) = delay else {
            if unreachable(kind) {
                return Err(SummaryEnd::Unreachable);
            }
            return Err(SummaryEnd::Failed(SummaryError::of(kind)));
        };
        tokio::select! {
            _ = operation.cancelled() => return Err(SummaryEnd::Cancelled),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// Summarises the transcript in one request, or section by section and then
/// merged when it is longer than `max_chars`. Chinese is written in the
/// script `hints.language` chose, whichever one Codex replied in.
pub async fn summarize(
    summarizer: &dyn Summarizer,
    policy: &RetryPolicy,
    turns: &[MeetingTurn],
    speakers: &[MeetingSpeaker],
    max_chars: usize,
    hints: Hints<'_>,
    operation: &ProcessingOperation,
) -> SummaryEnd {
    let lines = transcript_lines(turns, speakers);
    if lines.is_empty() {
        return SummaryEnd::Failed(SummaryError::NothingToSummarize);
    }
    let transcript = lines.join("\n");
    let result = if transcript.chars().count() <= max_chars {
        ask(
            summarizer,
            policy,
            with_vocabulary(instructions(), hints.vocabulary),
            transcript,
            operation,
        )
        .await
    } else {
        let parts = sections(&lines, max_chars.min(SECTION_CHARS));
        let count = parts.len();
        let mut notes = Vec::with_capacity(count);
        for (index, part) in parts.into_iter().enumerate() {
            match ask(
                summarizer,
                policy,
                with_vocabulary(section_instructions(), hints.vocabulary),
                part,
                operation,
            )
            .await
            {
                Ok(output) => notes.push(format!(
                    "Notes of part {} of {count}:\n\n{}",
                    index + 1,
                    output.trim()
                )),
                Err(end) => return end,
            }
        }
        ask(
            summarizer,
            policy,
            with_vocabulary(merge_instructions(), hints.vocabulary),
            notes.join("\n\n"),
            operation,
        )
        .await
    };
    match result.map(|output| parse(&in_chosen_script(hints.language, &output), speakers)) {
        Ok(Ok(summary)) => SummaryEnd::Written(summary),
        Ok(Err(error)) => SummaryEnd::Failed(error),
        Err(end) => end,
    }
}

/// How writing a summary went.
pub struct Summarized {
    /// False when cancelled, or when the meeting was transcribed again or
    /// re-analysed meanwhile.
    pub settled: bool,
    pub codex: CodexVerdict,
}

impl Summarized {
    fn not_asked() -> Self {
        Self {
            settled: true,
            codex: CodexVerdict::Unknown,
        }
    }
}

impl SummaryError {
    fn verdict(self) -> CodexVerdict {
        match self {
            Self::RateLimited => CodexVerdict::RateLimited,
            Self::TooLong | Self::Empty => CodexVerdict::Answered,
            Self::SignedOut
            | Self::Network
            | Self::Server
            | Self::Timeout
            | Self::NothingToSummarize
            | Self::Other => CodexVerdict::Unknown,
        }
    }
}

/// Stage D: writes the summary of a transcribed meeting that has none yet,
/// and names the meeting unless the user did.
pub async fn write(
    store: &MeetingsStore,
    summarizer: &dyn Summarizer,
    policy: &RetryPolicy,
    id: i64,
    hints: Hints<'_>,
    operation: &ProcessingOperation,
) -> anyhow::Result<Summarized> {
    let Some(meeting) = store.get(id)? else {
        return Ok(Summarized::not_asked());
    };
    if meeting.summary_status != SummaryStatus::Pending {
        return Ok(Summarized::not_asked());
    }
    let started = std::time::Instant::now();
    match summarize(
        summarizer,
        policy,
        &meeting.turns,
        &meeting.speakers,
        MAX_SUMMARY_CHARS,
        hints,
        operation,
    )
    .await
    {
        SummaryEnd::Written(summary) => {
            log::info!(
                "Meeting {id}: summary written in {:?} ({} characters)",
                started.elapsed(),
                summary.body.chars().count()
            );
            Ok(Summarized {
                settled: store.save_summary(id, summary.title.as_deref(), &summary.body)?,
                codex: CodexVerdict::Answered,
            })
        }
        SummaryEnd::Failed(error) => {
            log::warn!("Meeting {id}: no summary ({error:?})");
            Ok(Summarized {
                settled: store.summary_failed(id, error)?,
                codex: error.verdict(),
            })
        }
        // Still pending: the queue writes it once the connection is back,
        // and at the next launch.
        SummaryEnd::Unreachable => {
            log::warn!("Meeting {id}: no summary yet, Codex is out of reach");
            Ok(Summarized {
                settled: false,
                codex: CodexVerdict::Unreachable,
            })
        }
        SummaryEnd::Cancelled => Ok(Summarized {
            settled: false,
            codex: CodexVerdict::Unknown,
        }),
    }
}

#[cfg(test)]
mod tests;
