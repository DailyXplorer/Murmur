use super::*;

#[test]
fn a_transcribed_meeting_gets_its_summary_title_and_small_audio() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(vec![Ok(
        "# Point budget\n\n[[s1]] et [[me]] valident le budget.".into(),
    )]);
    h.process_all(
        id,
        codex(|_| Ok("Le budget est validé.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.title.as_deref(), Some("Point budget"));
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    assert_eq!(
        meeting.summary.as_deref(),
        Some("[[s1]] et [[me]] valident le budget.")
    );
    // Codex read the speaker-labelled transcript.
    let input = summarizer.inputs.lock().unwrap()[0].clone();
    assert!(input.contains("[[s1]] Speaker 2: Le budget est validé."));
    assert!(input.contains("[[me]] Me: Le budget est validé."));

    let dir = paths::meeting_dir(&h.root, id);
    if cfg!(target_arch = "aarch64") {
        assert_eq!(meeting.audio, AudioState::Compressed);
        assert!(dir.join(paths::AUDIO_FILE).exists());
        assert!(!dir.join(paths::MIC_TRACK).exists());
        assert!(!dir.join(paths::SYSTEM_TRACK).exists());
    }
    // Windows has no archive bridge: the tracks still wait to be compressed.
    if cfg!(target_os = "macos") {
        assert!(h.store.waiting_for_processing().unwrap().is_empty());
    }
}

#[test]
fn a_meeting_is_written_in_the_chinese_script_the_user_chose() {
    let h = Harness {
        language: Some("zh-Hant"),
        ..harness()
    };
    let id = spoken_meeting(&h);
    // Codex is told only "zh" and answers in Simplified characters.
    h.process_all(
        id,
        codex(|_| Ok("我们开会吧。".into())),
        Arc::new(FakeAnalyzer::without_models()),
        FakeSummarizer::answering(vec![Ok("# 会议记录\n\n[[me]] 说明了预算。".into())]),
    );

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(
        turns(&meeting),
        [("s1", "我們開會吧。"), ("me", "我們開會吧。")]
    );
    assert_eq!(meeting.title.as_deref(), Some("會議記錄"));
    assert_eq!(meeting.summary.as_deref(), Some("[[me]] 說明了預算。"));
}

#[test]
fn a_transcript_built_again_follows_the_chinese_script_chosen_since() {
    let mut h = harness();
    let id = spoken_meeting(&h);
    h.process(id, codex(|_| Ok("我们开会吧。".into())));

    // Separate Speakers builds the transcript again from the saved pieces.
    h.language = Some("zh-Hant");
    assert!(h.store.reopen(id).unwrap());
    let codex_again = codex(|_| Ok("never".into()));
    h.process(id, Arc::clone(&codex_again));

    assert!(codex_again.calls.lock().unwrap().is_empty());
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(
        turns(&meeting),
        [("s1", "我們開會吧。"), ("me", "我們開會吧。")]
    );
}

#[test]
fn a_failed_summary_keeps_the_meeting_ready_and_retry_writes_it() {
    let h = harness();
    let id = spoken_meeting(&h);
    h.store.rename(id, Some("Mon titre")).unwrap();
    let summarizer = FakeSummarizer::answering(lasting_summary_rate_limit());
    h.process_all(
        id,
        codex(|_| Ok("Bonjour.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.turns.len(), 2);
    assert_eq!(meeting.summary_status, SummaryStatus::Failed);
    assert_eq!(meeting.summary_error, Some(SummaryError::RateLimited));

    // Retry asks for the summary only.
    assert!(h.store.retry_summary(id).unwrap());
    let codex_again = codex(|_| Ok("never".into()));
    h.process_all(
        id,
        Arc::clone(&codex_again),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    assert!(codex_again.calls.lock().unwrap().is_empty());
    assert_eq!(summarizer.calls(), SUMMARY_ATTEMPTS + 1);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    assert_eq!(meeting.summary.as_deref(), Some("Notes."));
    // The user's title stays.
    assert_eq!(meeting.title.as_deref(), Some("Mon titre"));
}

fn rate_limited_summary() -> Result<String, CompletionError> {
    Err(CompletionError::Failed {
        kind: FailureKind::RateLimited,
        detail: String::new(),
    })
}

/// A summary request and its retries within the harness's rate-limit
/// budget.
const SUMMARY_ATTEMPTS: usize = 4;

/// Rate limits on every attempt of one summary.
fn lasting_summary_rate_limit() -> Vec<Result<String, CompletionError>> {
    (0..SUMMARY_ATTEMPTS)
        .map(|_| rate_limited_summary())
        .collect()
}

#[test]
fn a_brief_summary_rate_limit_is_waited_out_and_does_not_hold_the_queue() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer =
        FakeSummarizer::answering(vec![rate_limited_summary(), Ok("# Réunion\nNotes.".into())]);
    let verdict = h.process_all(
        id,
        codex(|_| Ok("Bonjour.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    assert_eq!(verdict, CodexVerdict::Answered);
    assert_eq!(summarizer.calls(), 2);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
}

#[test]
fn a_summary_codex_cannot_reach_stays_pending_for_the_queue() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(vec![Err(CompletionError::Failed {
        kind: FailureKind::Network,
        detail: String::new(),
    })]);
    let verdict = h.process_all(
        id,
        codex(|_| Ok("Bonjour.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    assert_eq!(verdict, CodexVerdict::Unreachable);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.summary_error, None);
    // Compressed once the summary is settled.
    assert_eq!(meeting.audio, AudioState::Tracks);
    assert!(h.store.waiting_for_processing().unwrap().contains(&id));
}

#[test]
fn an_offline_summary_after_rate_limited_local_fallback_stays_in_the_live_queue() {
    use crate::meetings::{processing_finished, Next, ProcessingQueue};
    use std::time::SystemTime;

    let local = codex(|_| Ok("Sur ce Mac.".into()));
    let h = Harness {
        local: Some(Arc::clone(&local) as Arc<dyn Transcriber>),
        ..harness()
    };
    let id = spoken_meeting(&h);
    let queue = ProcessingQueue::new(false);
    queue.enqueue(id);
    assert_eq!(
        queue.state().take_next(SystemTime::now(), true),
        Next::Process(id)
    );
    let summarizer = FakeSummarizer::answering(vec![Err(CompletionError::Failed {
        kind: FailureKind::Network,
        detail: String::new(),
    })]);
    let verdict = h.process_all(
        id,
        codex(|_| Err(FailureKind::RateLimited)),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    assert_eq!(verdict, CodexVerdict::RateLimited);
    assert_eq!(summarizer.calls(), 1);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.audio, AudioState::Tracks);
    let local_calls = local.calls.lock().unwrap().len();
    assert!(local_calls > 0);

    processing_finished(&queue, &h.store, id, verdict, true);
    assert_eq!(queue.queued(), [id]);
    assert!(queue.held_for(id).is_some());
    let until = queue.state().held_until.unwrap();
    assert_eq!(queue.state().take_next(until, true), Next::Process(id));

    let cloud_again = codex(|_| panic!("completed chunks must not be sent again"));
    let recovered = FakeSummarizer::answering(vec![Ok("# Recovered\nNotes.".into())]);
    let verdict = h.process_all(
        id,
        Arc::clone(&cloud_again),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&recovered),
    );
    processing_finished(&queue, &h.store, id, verdict, true);
    assert_eq!(verdict, CodexVerdict::Answered);
    assert!(queue.queued().is_empty());
    assert_eq!(recovered.calls(), 1);
    assert!(cloud_again.calls.lock().unwrap().is_empty());
    assert_eq!(local.calls.lock().unwrap().len(), local_calls);
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.summary_status, SummaryStatus::Ready);
    // Windows has no archive bridge to compress it.
    if cfg!(target_os = "macos") {
        assert_eq!(meeting.audio, AudioState::Compressed);
    }
}

#[test]
fn a_meeting_codex_transcribes_says_codex_answered() {
    let h = harness();
    let id = spoken_meeting(&h);
    let verdict = h.process_all(
        id,
        codex(|_| Ok("Bonjour.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        FakeSummarizer::answering(vec![Ok("Notes.".into())]),
    );
    assert_eq!(verdict, CodexVerdict::Answered);
}

#[test]
fn past_the_rate_limit_a_ready_model_fills_the_meeting_and_counts_its_time() {
    let local = codex(|seconds| Ok(format!("Sur ce Mac, {seconds} secondes.")));
    let h = Harness {
        local: Some(Arc::clone(&local) as Arc<dyn Transcriber>),
        ..harness()
    };
    let id = spoken_meeting(&h);
    let verdict = h.process_with(
        id,
        codex(|_| Err(FailureKind::RateLimited)),
        Arc::new(FakeAnalyzer::without_models()),
    );

    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.error, None);
    assert!(meeting
        .turns
        .iter()
        .all(|turn| turn.text.contains("Sur ce Mac")));
    let spoken_ms: i64 = h
        .store
        .chunks(id)
        .unwrap()
        .iter()
        .filter(|chunk| chunk.status == ChunkStatus::Done)
        .map(|chunk| (chunk.end_ms - chunk.start_ms) as i64)
        .sum();
    assert_eq!(local.calls.lock().unwrap().len(), 2);
    assert!(spoken_ms > 0);
    assert_eq!(meeting.local_fallback_ms, spoken_ms);
    // Codex still limits: the meetings queued after this one wait.
    assert_eq!(verdict, CodexVerdict::RateLimited);

    // Transcribing again with Codex leaves nothing done on this Mac.
    assert!(h.store.retranscribe(id).unwrap());
    let h = Harness { local: None, ..h };
    h.process(id, codex(|_| Ok("Par Codex.".into())));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.local_fallback_ms, 0);
}

#[test]
fn past_the_rate_limit_a_language_the_model_lacks_waits_for_codex() {
    let local = codex(|_| Ok("サンプル".into()));
    let h = Harness {
        language: Some("ja"),
        local: Some(Arc::clone(&local) as Arc<dyn Transcriber>),
        ..harness()
    };
    let id = spoken_meeting(&h);
    let verdict = h.process_with(
        id,
        codex(|_| Err(FailureKind::RateLimited)),
        Arc::new(FakeAnalyzer::without_models()),
    );

    assert!(local.calls.lock().unwrap().is_empty());
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.error, Some(ProcessingError::RateLimited));
    assert_eq!(meeting.local_fallback_ms, 0);
    assert_eq!(verdict, CodexVerdict::RateLimited);
}

#[test]
fn a_rate_limited_transcription_says_codex_is_limiting() {
    let h = harness();
    let id = spoken_meeting(&h);
    let verdict = h.process_with(
        id,
        codex(|_| Err(FailureKind::RateLimited)),
        Arc::new(FakeAnalyzer::without_models()),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.error, Some(ProcessingError::RateLimited));
    assert_eq!(verdict, CodexVerdict::RateLimited);
}

#[test]
fn a_run_that_ran_out_of_rate_limit_budget_holds_the_queue_after_earlier_answers() {
    let h = harness();
    let id = spoken_meeting(&h);
    let saver = Saver {
        id,
        store: Arc::clone(&h.store),
        events: Arc::clone(&h.events) as Arc<dyn ProcessEvents>,
        done: AtomicU32::new(0),
        total: 2,
        rate_limited: AtomicBool::new(false),
        unreachable: AtomicBool::new(false),
        answered: AtomicBool::new(false),
    };
    let job = |track| Job {
        track,
        idx: 0,
        start_ms: 0,
        end_ms: 20_000,
    };
    saver.finished(&job(Track::System), &Outcome::Done("Bonjour.".into()));
    assert_eq!(saver.verdict(), CodexVerdict::Answered);
    // The run then ends on something else, such as an unreadable track.
    saver.finished(&job(Track::Mic), &Outcome::Failed(FailureKind::RateLimited));
    assert_eq!(saver.verdict(), CodexVerdict::RateLimited);
}

#[test]
fn a_summary_retry_codex_refuses_says_codex_is_limiting() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(
        (0..2 * SUMMARY_ATTEMPTS)
            .map(|_| rate_limited_summary())
            .collect(),
    );
    let verdict = h.process_all(
        id,
        codex(|_| Ok("Bonjour.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    // The words came through, but Codex refused right after.
    assert_eq!(verdict, CodexVerdict::RateLimited);

    // The meeting stays Ready, and no part is transcribed again.
    assert!(h.store.retry_summary(id).unwrap());
    let codex_again = codex(|_| Ok("never".into()));
    let verdict = h.process_all(
        id,
        Arc::clone(&codex_again),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    assert!(codex_again.calls.lock().unwrap().is_empty());
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.summary_error, Some(SummaryError::RateLimited));
    assert_eq!(verdict, CodexVerdict::RateLimited);
}

#[test]
fn transcribing_again_while_the_summary_is_written_drops_that_summary() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(vec![
        Ok("# Old title\nOld notes.".into()),
        Ok("# New title\nNew notes.".into()),
    ]);
    let store = Arc::clone(&h.store);
    *summarizer.meanwhile.lock().unwrap() = Some(Box::new(move || {
        assert!(store.retranscribe(id).unwrap());
    }));
    h.process_all(
        id,
        codex(|_| Ok("Bonjour.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Processing);
    assert_eq!(meeting.summary, None);
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(meeting.title, None);
    // The run stopped there: the tracks wait for the new transcript.
    assert_eq!(meeting.audio, AudioState::Tracks);
    let dir = paths::meeting_dir(&h.root, id);
    assert!(dir.join(paths::MIC_TRACK).exists());

    h.process_all(
        id,
        codex(|_| Ok("Au revoir.".into())),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    assert_eq!(summarizer.calls(), 2);
    assert!(summarizer.inputs.lock().unwrap()[1].contains("Au revoir."));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert_eq!(meeting.summary.as_deref(), Some("New notes."));
    assert_eq!(meeting.title.as_deref(), Some("New title"));
}

#[test]
fn no_summary_is_written_while_parts_are_missing() {
    let h = harness();
    let id = spoken_meeting(&h);
    let summarizer = FakeSummarizer::answering(vec![]);
    h.process_all(
        id,
        codex(|_| Err(FailureKind::Rejected)),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Failed);
    assert_eq!(meeting.summary_status, SummaryStatus::Pending);
    assert_eq!(summarizer.calls(), 0);
    // The tracks stay for the retry.
    assert_eq!(meeting.audio, AudioState::Tracks);
    assert!(paths::meeting_dir(&h.root, id)
        .join(paths::MIC_TRACK)
        .exists());
}

#[cfg(target_arch = "aarch64")]
#[test]
fn a_meeting_is_transcribed_again_from_its_compressed_audio() {
    let h = harness();
    let id = spoken_meeting(&h);
    h.process(id, codex(|_| Ok("Première version.".into())));
    let dir = paths::meeting_dir(&h.root, id);
    assert!(!dir.join(paths::MIC_TRACK).exists());

    assert!(h.store.retranscribe(id).unwrap());
    let summarizer = FakeSummarizer::answering(vec![Ok("# Nouveau\nNotes.".into())]);
    let again = codex(|seconds| Ok(format!("Version de {seconds} secondes.")));
    h.process_all(
        id,
        Arc::clone(&again),
        Arc::new(FakeAnalyzer::without_models()),
        Arc::clone(&summarizer),
    );
    // Both tracks were decoded at their full length and sent again.
    let sent = again.calls.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|seconds| (19..=20).contains(seconds)));
    let meeting = h.store.get(id).unwrap().unwrap();
    assert_eq!(meeting.status, MeetingStatus::Ready);
    assert!(meeting
        .turns
        .iter()
        .all(|turn| turn.text.contains("Version")));
    assert_eq!(meeting.title.as_deref(), Some("Nouveau"));
    if cfg!(target_os = "macos") {
        assert_eq!(meeting.audio, AudioState::Compressed);
        assert!(!dir.join(paths::MIC_TRACK).exists());
    }
}
