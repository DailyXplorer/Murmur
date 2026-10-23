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
/// script `language` chose, whichever one Codex replied in.
pub async fn summarize(
    summarizer: &dyn Summarizer,
    policy: &RetryPolicy,
    turns: &[MeetingTurn],
    speakers: &[MeetingSpeaker],
    max_chars: usize,
    language: Option<&str>,
    operation: &ProcessingOperation,
) -> SummaryEnd {
    let lines = transcript_lines(turns, speakers);
    if lines.is_empty() {
        return SummaryEnd::Failed(SummaryError::NothingToSummarize);
    }
    let transcript = lines.join("\n");
    let result = if transcript.chars().count() <= max_chars {
        ask(summarizer, policy, instructions(), transcript, operation).await
    } else {
        let parts = sections(&lines, max_chars.min(SECTION_CHARS));
        let count = parts.len();
        let mut notes = Vec::with_capacity(count);
        for (index, part) in parts.into_iter().enumerate() {
            match ask(summarizer, policy, section_instructions(), part, operation).await {
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
            merge_instructions(),
            notes.join("\n\n"),
            operation,
        )
        .await
    };
    match result.map(|output| parse(&in_chosen_script(language, &output), speakers)) {
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
    language: Option<&str>,
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
        language,
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
mod tests {
    use super::*;
    use crate::OperationId;
    use std::sync::Mutex;

    #[test]
    fn a_summary_connects_on_any_link_a_transcription_connects_on() {
        let request = summary_request("model", "instructions", "transcript");
        assert!(request.connect_timeout >= crate::codex_transcribe::CONNECT_TIMEOUT);
    }

    fn speaker(key: &str, name: &str) -> MeetingSpeaker {
        MeetingSpeaker {
            key: key.into(),
            name: name.into(),
            default_name: name.into(),
            color_index: 0,
        }
    }

    fn turn(key: &str, start_ms: i64, text: &str) -> MeetingTurn {
        MeetingTurn {
            speaker_key: key.into(),
            start_ms,
            end_ms: start_ms + 1_000,
            text: text.into(),
            low_confidence: false,
        }
    }

    fn meeting() -> (Vec<MeetingSpeaker>, Vec<MeetingTurn>) {
        (
            vec![speaker("me", "Me"), speaker("s1", "Camille")],
            vec![
                turn("s1", 2_000, "On fait le point sur le budget ?"),
                turn("me", 5_000, " Oui, il est validé. "),
                turn(PLACEHOLDER, 600_000, ""),
                turn("s1", 3_725_000, "Je relance l'équipe client demain."),
            ],
        )
    }

    #[test]
    fn the_transcript_is_sent_with_times_markers_and_names() {
        let (speakers, turns) = meeting();
        assert_eq!(
            transcript_lines(&turns, &speakers),
            [
                "[00:02] [[s1]] Camille: On fait le point sur le budget ?",
                "[00:05] [[me]] Me: Oui, il est validé.",
                "[1:02:05] [[s1]] Camille: Je relance l'équipe client demain.",
            ]
        );
    }

    #[test]
    fn instructions_ask_for_the_shape_and_markers() {
        let text = instructions();
        for needle in [
            "# <title of at most 8 words>",
            "**Action items**",
            "[[me]] or [[s2]]",
            "The title is about the topic only",
            "language most of the transcript",
        ] {
            assert!(text.contains(needle), "{needle}");
        }
        assert!(!merge_instructions().contains(TRANSCRIPT_FORMAT));
    }

    #[test]
    fn the_title_line_and_the_body_are_read_apart() {
        let output = "# Point budget et planning\n\nRéunion de suivi entre [[me]] et [[s1]].\n\n**Budget**\n- Validé par [[s1]]\n";
        assert_eq!(
            parse(output, &[]),
            Ok(Summary {
                title: Some("Point budget et planning".into()),
                body: "Réunion de suivi entre [[me]] et [[s1]].\n\n**Budget**\n- Validé par [[s1]]"
                    .into(),
            })
        );
    }

    #[test]
    fn code_fences_are_removed_and_a_missing_title_stays_missing() {
        let output = "```markdown\nWeekly sync between [[me]] and [[s2]].\n\n**Hiring**\n- Two offers out\n```";
        assert_eq!(
            parse(output, &[]),
            Ok(Summary {
                title: None,
                body: "Weekly sync between [[me]] and [[s2]].\n\n**Hiring**\n- Two offers out"
                    .into(),
            })
        );
    }

    #[test]
    fn lists_are_kept_to_two_levels() {
        let output = "# T\n**Plan**\n* one\n    - two\n      + three\n- four";
        assert_eq!(
            parse(output, &[]).unwrap().body,
            "**Plan**\n- one\n  - two\n  - three\n- four"
        );
    }

    #[test]
    fn a_reply_without_a_body_is_empty() {
        assert_eq!(parse("# Only a title\n\n", &[]), Err(SummaryError::Empty));
        assert_eq!(parse("  \n", &[]), Err(SummaryError::Empty));
        assert_eq!(parse("```\n```", &[]), Err(SummaryError::Empty));
    }

    #[test]
    fn a_long_title_is_cut() {
        let output = format!("# {}\nBody.", "a".repeat(200));
        assert_eq!(
            parse(&output, &[]).unwrap().title.unwrap().chars().count(),
            MAX_TITLE_CHARS
        );
    }

    #[test]
    fn markers_become_names_and_unknown_ones_their_key() {
        let names: HashMap<&str, String> =
            [("me", "Me".to_string()), ("s1", "Camille".to_string())].into();
        assert_eq!(
            resolve_markers(
                "[[me]] asked [[s1]]; [[s9]] was away. [[not a key]] [[",
                &names
            ),
            "Me asked Camille; s9 was away. [[not a key]] [["
        );
    }

    /// Answers each request in turn and keeps what it was sent.
    struct Script {
        answers: Mutex<Vec<Result<String, CompletionError>>>,
        requests: Mutex<Vec<(String, String)>>,
    }

    impl Script {
        fn new(answers: Vec<Result<String, CompletionError>>) -> Self {
            Self {
                answers: Mutex::new(answers),
                requests: Mutex::default(),
            }
        }
    }

    impl Summarizer for Script {
        fn complete(
            &self,
            instructions: String,
            input: String,
            _: ProcessingOperation,
        ) -> CompleteFuture {
            self.requests.lock().unwrap().push((instructions, input));
            let answer = self.answers.lock().unwrap().remove(0);
            Box::pin(async move { answer })
        }
    }

    /// Retries at once, three times for a transient failure and three times
    /// for a rate limit.
    fn quick_policy() -> RetryPolicy {
        RetryPolicy {
            rate_limit_first: Duration::from_millis(1),
            rate_limit_cap: Duration::from_millis(1),
            rate_limit_budget: Duration::from_millis(3),
            transient: vec![Duration::ZERO; 3],
        }
    }

    fn no_retries() -> RetryPolicy {
        RetryPolicy {
            rate_limit_budget: Duration::ZERO,
            transient: vec![],
            ..quick_policy()
        }
    }

    fn run_with(script: &dyn Summarizer, policy: &RetryPolicy, max_chars: usize) -> SummaryEnd {
        let (speakers, turns) = meeting();
        tauri::async_runtime::block_on(summarize(
            script,
            policy,
            &turns,
            &speakers,
            max_chars,
            None,
            &ProcessingOperation::new(OperationId(1)),
        ))
    }

    fn run(script: &dyn Summarizer, max_chars: usize) -> SummaryEnd {
        run_with(script, &no_retries(), max_chars)
    }

    fn failed(kind: FailureKind) -> Result<String, CompletionError> {
        Err(CompletionError::Failed {
            kind,
            detail: String::new(),
        })
    }

    #[test]
    fn brief_rate_limits_and_dropped_streams_are_retried() {
        let script = Script::new(vec![
            failed(FailureKind::RateLimited),
            failed(FailureKind::Network),
            failed(FailureKind::ServerError),
            failed(FailureKind::Timeout),
            failed(FailureKind::RateLimited),
            Ok("# Budget\nContexte.".into()),
        ]);
        assert_eq!(
            run_with(&script, &quick_policy(), MAX_SUMMARY_CHARS),
            SummaryEnd::Written(Summary {
                title: Some("Budget".into()),
                body: "Contexte.".into()
            })
        );
        assert_eq!(script.requests.lock().unwrap().len(), 6);
    }

    #[test]
    fn a_lasting_rate_limit_fails_the_summary_once_its_budget_is_spent() {
        let script = Script::new((0..4).map(|_| failed(FailureKind::RateLimited)).collect());
        assert_eq!(
            run_with(&script, &quick_policy(), MAX_SUMMARY_CHARS),
            SummaryEnd::Failed(SummaryError::RateLimited)
        );
        assert_eq!(script.requests.lock().unwrap().len(), 4);
    }

    #[test]
    fn a_lasting_network_failure_leaves_codex_out_of_reach_after_three_retries() {
        let script = Script::new((0..4).map(|_| failed(FailureKind::Network)).collect());
        assert_eq!(
            run_with(&script, &quick_policy(), MAX_SUMMARY_CHARS),
            SummaryEnd::Unreachable
        );
        assert_eq!(script.requests.lock().unwrap().len(), 4);
    }

    /// Codex was reached: the summary does not say it could not be.
    #[test]
    fn a_lasting_server_failure_or_cut_reply_fails_as_codex_trouble() {
        for kind in [FailureKind::ServerError, FailureKind::ConnectionDropped] {
            let script = Script::new((0..4).map(|_| failed(kind)).collect());
            assert_eq!(
                run_with(&script, &quick_policy(), MAX_SUMMARY_CHARS),
                SummaryEnd::Failed(SummaryError::Server),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_refused_request_is_not_retried() {
        for kind in [FailureKind::SignedOut, FailureKind::Rejected] {
            let script = Script::new(vec![failed(kind)]);
            run_with(&script, &quick_policy(), MAX_SUMMARY_CHARS);
            assert_eq!(script.requests.lock().unwrap().len(), 1, "{kind:?}");
        }
    }

    #[test]
    fn cancelling_during_a_retry_wait_cancels_the_summary() {
        let script = Script::new(vec![failed(FailureKind::RateLimited)]);
        let operation = ProcessingOperation::new(OperationId(1));
        let policy = RetryPolicy {
            rate_limit_first: Duration::from_secs(600),
            rate_limit_cap: Duration::from_secs(600),
            rate_limit_budget: Duration::from_secs(3600),
            transient: vec![],
        };
        let (speakers, turns) = meeting();
        let end = tauri::async_runtime::block_on(async {
            let summary = summarize(
                &script,
                &policy,
                &turns,
                &speakers,
                MAX_SUMMARY_CHARS,
                None,
                &operation,
            );
            let cancel = async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                operation.cancel();
            };
            tokio::join!(summary, cancel).0
        });
        assert_eq!(end, SummaryEnd::Cancelled);
    }

    #[test]
    fn a_meeting_that_fits_is_summarised_in_one_request() {
        let script = Script::new(vec![Ok("# Budget\nContexte.".into())]);
        assert_eq!(
            run(&script, MAX_SUMMARY_CHARS),
            SummaryEnd::Written(Summary {
                title: Some("Budget".into()),
                body: "Contexte.".into()
            })
        );
        assert_eq!(script.requests.lock().unwrap().len(), 1);
    }

    /// A marker Codex put in the title despite the rules becomes a name.
    #[test]
    fn a_marker_in_the_title_becomes_the_speaker_s_name() {
        let script = Script::new(vec![Ok(
            "# Entretien avec [[s1]]\nPoint entre [[me]] et [[s1]].".into(),
        )]);
        assert_eq!(
            run(&script, MAX_SUMMARY_CHARS),
            SummaryEnd::Written(Summary {
                title: Some("Entretien avec Camille".into()),
                body: "Point entre [[me]] et [[s1]].".into()
            })
        );
        // Named before the title is cut, so a marker at the limit is not split.
        let start = "a".repeat(MAX_TITLE_CHARS - 4);
        let script = Script::new(vec![Ok(format!("# {start} [[s1]]\nCorps."))]);
        let SummaryEnd::Written(summary) = run(&script, MAX_SUMMARY_CHARS) else {
            panic!("no summary");
        };
        assert_eq!(summary.title, Some(format!("{start} Cam")));
    }

    #[test]
    fn a_long_meeting_is_summarised_by_section_then_merged() {
        let script = Script::new(vec![
            Ok("# A\nPartie un.".into()),
            Ok("# B\nPartie deux.".into()),
            Ok("# Budget et relances\nTout.".into()),
        ]);
        // Two lines fit in one section, the third starts another.
        assert_eq!(
            run(&script, 100),
            SummaryEnd::Written(Summary {
                title: Some("Budget et relances".into()),
                body: "Tout.".into()
            })
        );
        let requests = script.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].1.contains("[[s1]] Camille: On fait"));
        assert!(requests[0].1.contains("[[me]] Me: Oui"));
        assert!(requests[1].1.contains("Je relance"));
        assert_eq!(requests[0].0, section_instructions());
        assert_eq!(requests[2].0, merge_instructions());
        assert!(requests[2]
            .1
            .contains("Notes of part 2 of 2:\n\n# B\nPartie deux."));
    }

    #[test]
    fn failures_are_told_apart() {
        for (answer, end) in [
            (
                failed(FailureKind::SignedOut),
                SummaryEnd::Failed(SummaryError::SignedOut),
            ),
            (
                failed(FailureKind::RateLimited),
                SummaryEnd::Failed(SummaryError::RateLimited),
            ),
            (
                failed(FailureKind::ServerError),
                SummaryEnd::Failed(SummaryError::Network),
            ),
            (
                Err(CompletionError::TooLong),
                SummaryEnd::Failed(SummaryError::TooLong),
            ),
            (Err(CompletionError::Cancelled), SummaryEnd::Cancelled),
            (Ok("\n".into()), SummaryEnd::Failed(SummaryError::Empty)),
            (failed(FailureKind::Network), SummaryEnd::Unreachable),
            // Codex was reached: a long summary that runs out of time is no
            // reason to wait for the connection.
            (
                failed(FailureKind::Timeout),
                SummaryEnd::Failed(SummaryError::Timeout),
            ),
        ] {
            assert_eq!(run(&Script::new(vec![answer]), MAX_SUMMARY_CHARS), end);
        }
        // A failed section stops before the merge.
        let script = Script::new(vec![failed(FailureKind::Network)]);
        assert_eq!(run(&script, 100), SummaryEnd::Unreachable);
        assert_eq!(script.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_meeting_with_no_words_is_not_sent() {
        let script = Script::new(vec![]);
        let end = tauri::async_runtime::block_on(summarize(
            &script,
            &no_retries(),
            &[turn(PLACEHOLDER, 0, "")],
            &[],
            MAX_SUMMARY_CHARS,
            None,
            &ProcessingOperation::new(OperationId(1)),
        ));
        assert_eq!(end, SummaryEnd::Failed(SummaryError::NothingToSummarize));
    }

    /// Answers each connection with the next canned HTTP response and keeps
    /// the request bodies.
    fn fake_codex(responses: Vec<String>) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let bodies = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&bodies);
        std::thread::spawn(move || {
            for response in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&body).into_owned());
                let mut stream = stream;
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (url, bodies)
    }

    fn http(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn sse(text: &str) -> String {
        let delta = serde_json::json!({"type": "response.output_text.delta", "delta": text});
        http(
            "200 OK",
            "text/event-stream",
            &format!("data: {delta}\n\ndata: {{\"type\":\"response.completed\"}}\n\n"),
        )
    }

    #[test]
    fn codex_is_asked_with_the_standard_model_and_a_retry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("auth.json"),
            serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": {"access_token": "token", "refresh_token": "refresh", "account_id": "acct"},
                "last_refresh": chrono::Utc::now().to_rfc3339(),
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("models_cache.json"),
            r#"{"models":[{"slug":"gpt-6-luna","priority":0},{"slug":"gpt-6.2-sol","priority":1}]}"#,
        )
        .unwrap();
        let listed = http(
            "200 OK",
            "application/json",
            &std::fs::read_to_string(dir.path().join("models_cache.json")).unwrap(),
        );
        let (url, bodies) = fake_codex(vec![
            listed.clone(),
            http("429 Too Many Requests", "application/json", "{}"),
            listed,
            sse("# Budget\n\nRéunion entre [[me]] et [[s1]]."),
        ]);
        let codex = CodexSummarizer {
            url,
            auth_path: Some(dir.path().join("auth.json")),
        };
        assert_eq!(
            run(&codex, MAX_SUMMARY_CHARS),
            SummaryEnd::Failed(SummaryError::RateLimited)
        );
        // Retry.
        assert_eq!(
            run(&codex, MAX_SUMMARY_CHARS),
            SummaryEnd::Written(Summary {
                title: Some("Budget".into()),
                body: "Réunion entre [[me]] et [[s1]].".into()
            })
        );
        let request: serde_json::Value = serde_json::from_str(&bodies.lock().unwrap()[3]).unwrap();
        assert_eq!(request["model"], "gpt-6.2-sol");
        assert_eq!(request["reasoning"]["effort"], "medium");
        assert_eq!(request["text"]["verbosity"], "medium");
        assert!(request["input"][1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[00:05] [[me]] Me: Oui, il est validé."));
    }

    fn signed_in(dir: &std::path::Path, cached_models: &str) -> PathBuf {
        std::fs::write(
            dir.join("auth.json"),
            serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": {"access_token": "token", "refresh_token": "refresh", "account_id": "acct"},
                "last_refresh": chrono::Utc::now().to_rfc3339(),
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("models_cache.json"), cached_models).unwrap();
        dir.join("auth.json")
    }

    fn model_of(body: &str) -> String {
        let request: serde_json::Value = serde_json::from_str(body).unwrap();
        request["model"].as_str().unwrap().to_string()
    }

    /// The cache still lists the models of a plan the account left; the
    /// server's own list is the one that counts.
    #[test]
    fn the_standard_model_comes_from_the_account_s_live_list() {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = signed_in(
            dir.path(),
            r#"{"client_version":"0.159.0","models":[{"slug":"gpt-test-paid-sol","priority":1},{"slug":"gpt-test-free-luna","priority":4}]}"#,
        );
        let (url, bodies) = fake_codex(vec![
            http(
                "200 OK",
                "application/json",
                r#"{"models":[{"slug":"gpt-test-free-luna","priority":4},{"slug":"gpt-test-free-terra","priority":8}]}"#,
            ),
            sse("# Budget\n\nRéunion."),
        ]);
        let codex = CodexSummarizer {
            url,
            auth_path: Some(auth_path),
        };
        assert!(matches!(
            run(&codex, MAX_SUMMARY_CHARS),
            SummaryEnd::Written(_)
        ));
        assert_eq!(model_of(&bodies.lock().unwrap()[1]), "gpt-test-free-terra");
    }

    /// The live list is unavailable and the cached one names a model the
    /// plan lacks: the fast model writes the summary, now and for the rest
    /// of the session.
    #[test]
    fn a_model_the_plan_refuses_falls_back_to_the_fast_one_for_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let auth_path = signed_in(
            dir.path(),
            r#"{"models":[{"slug":"gpt-test-gone-sol","priority":1},{"slug":"gpt-test-gone-luna","priority":4}]}"#,
        );
        let unavailable = http("503 Service Unavailable", "application/json", "{}");
        let (url, bodies) = fake_codex(vec![
            unavailable.clone(),
            http(
                "400 Bad Request",
                "application/json",
                r#"{"detail":"The 'gpt-test-gone-sol' model is not supported when using Codex with a ChatGPT account."}"#,
            ),
            sse("# Budget\n\nRéunion."),
            unavailable,
            sse("# Budget\n\nRéunion."),
        ]);
        let codex = CodexSummarizer {
            url,
            auth_path: Some(auth_path),
        };
        for _ in 0..2 {
            assert!(matches!(
                run(&codex, MAX_SUMMARY_CHARS),
                SummaryEnd::Written(_)
            ));
        }
        let bodies = bodies.lock().unwrap();
        assert_eq!(model_of(&bodies[1]), "gpt-test-gone-sol");
        assert_eq!(model_of(&bodies[2]), "gpt-test-gone-luna");
        assert_eq!(model_of(&bodies[4]), "gpt-test-gone-luna");
    }

    #[test]
    fn errors_round_trip_through_the_database() {
        for error in [
            SummaryError::SignedOut,
            SummaryError::RateLimited,
            SummaryError::Network,
            SummaryError::Server,
            SummaryError::Timeout,
            SummaryError::TooLong,
            SummaryError::Empty,
            SummaryError::NothingToSummarize,
            SummaryError::Other,
        ] {
            assert_eq!(SummaryError::from_db(error.as_db()), Some(error));
        }
    }
}
