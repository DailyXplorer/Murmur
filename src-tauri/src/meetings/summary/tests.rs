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
    let output =
        "```markdown\nWeekly sync between [[me]] and [[s2]].\n\n**Hiring**\n- Two offers out\n```";
    assert_eq!(
        parse(output, &[]),
        Ok(Summary {
            title: None,
            body: "Weekly sync between [[me]] and [[s2]].\n\n**Hiring**\n- Two offers out".into(),
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
        Hints::default(),
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
            Hints::default(),
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

fn words(terms: &[&str]) -> Vec<String> {
    terms.iter().map(|term| term.to_string()).collect()
}

#[test]
fn an_empty_vocabulary_adds_no_line() {
    assert_eq!(vocabulary_line(&[]), None);
    assert_eq!(vocabulary_line(&words(&["", "  "])), None);
    assert_eq!(with_vocabulary(instructions(), &[]), instructions());
}

#[test]
fn the_vocabulary_line_lists_each_term_once_in_order() {
    let line = vocabulary_line(&words(&["Murmur", " Zoé ", "Murmur", "Open  AI"])).unwrap();
    assert!(line.starts_with("Names and terms that may appear: Murmur, Zoé, Open AI. "));
}

#[test]
fn the_vocabulary_line_stops_at_the_cap() {
    let many: Vec<String> = (0..MAX_VOCABULARY_TERMS + 20)
        .map(|n| format!("Term{n}"))
        .collect();
    let line = vocabulary_line(&many).unwrap();
    assert_eq!(line.matches("Term").count(), MAX_VOCABULARY_TERMS);
    assert!(line.contains(&format!("Term{}", MAX_VOCABULARY_TERMS - 1)));
    assert!(!line.contains(&format!("Term{MAX_VOCABULARY_TERMS},")));
    // Duplicates do not use up the cap.
    let mut repeated = vec!["Same".to_string(); 40];
    repeated.push("Last".into());
    assert!(vocabulary_line(&repeated).unwrap().contains("Same, Last."));
}

#[test]
fn every_request_of_a_long_meeting_carries_the_vocabulary() {
    let script = Script::new(vec![
        Ok("# A\nPartie un.".into()),
        Ok("# B\nPartie deux.".into()),
        Ok("# C\nTout.".into()),
    ]);
    let (speakers, turns) = meeting();
    let vocabulary = words(&["Murmur", "Zoé"]);
    let end = tauri::async_runtime::block_on(summarize(
        &script,
        &no_retries(),
        &turns,
        &speakers,
        100,
        Hints {
            language: None,
            vocabulary: &vocabulary,
        },
        &ProcessingOperation::new(OperationId(1)),
    ));
    assert!(matches!(end, SummaryEnd::Written(_)));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    for (instructions, input) in requests.iter() {
        assert!(instructions.ends_with("Names and terms that may appear: Murmur, Zoé. Spell them this way, in the title as well, when the transcript refers to them."));
        assert!(!input.contains("Names and terms"));
    }
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
            SummaryEnd::Failed(SummaryError::Server),
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
        Hints::default(),
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
