use super::{
    captured_audio, cleanup_plan, complete_unless_cancelled, context_reads, deliver,
    pad_short_capture, paste_outcome_commits_history, settle_wav, CleanupPlan, PendingWav,
    SHORT_DICTATION_WORDS,
};
use crate::audio_toolkit::text_passes::PassLanguage;
use crate::cleanup::{CleanupOutcome, ProcessedText, PromptContext};
use crate::clipboard::PasteOutcome;
use crate::field_excerpt::FieldExcerpt;
use crate::insights::categories::AppCategory;
use crate::settings::{get_default_settings, AppSettings, CategoryStyle, DictationStyle};
use crate::window_names::{ContextReads, WindowContext};
use std::cell::Cell;
use std::fs;
use std::future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

#[test]
fn a_cancel_keeps_the_captured_audio_not_the_padding() {
    let (padded, captured) = pad_short_capture(vec![0.5; 8_000]);
    assert_eq!(padded.len(), 20_000);
    let kept = captured_audio(Arc::new(padded), captured);
    assert_eq!(kept, vec![0.5; 8_000]);
    assert!(
        kept.len() < crate::recently_cancelled::MIN_SAMPLES,
        "a half-second dictation is too short for Restore, padded or not"
    );

    let (samples, captured) = pad_short_capture(vec![0.5; 24_000]);
    assert_eq!(samples.len(), 24_000);
    assert_eq!(captured_audio(Arc::new(samples), captured).len(), 24_000);
}

#[test]
fn cleanup_follows_the_chosen_dictation_style() {
    for style in [
        DictationStyle::Formal,
        DictationStyle::Normal,
        DictationStyle::Casual,
    ] {
        let mut settings = get_default_settings();
        settings.dictation_style = style;
        assert_eq!(
            request(&settings, true, WindowContext::default()).map(|context| context.style),
            Some(style)
        );
    }
}

#[test]
fn the_window_is_read_only_with_cleanup_and_the_setting_on() {
    for (setting, cleanup, read) in [
        (true, true, true),
        (false, true, false),
        (true, false, false),
        (false, false, false),
    ] {
        let mut settings = get_default_settings();
        settings.window_names_enabled = setting;
        settings.ai_cleanup_enabled = cleanup;
        assert_eq!(
            context_reads(&settings),
            ContextReads {
                kind: false,
                names: read,
                field: read
            },
            "setting {setting}, cleanup {cleanup}"
        );
    }
}

fn window() -> WindowContext {
    WindowContext {
        kind: None,
        names: vec!["Ingrid".to_string()],
        field: Some(FieldExcerpt {
            before: "Hi Ingrid, ".to_string(),
            after: String::new(),
        }),
    }
}

const LONG: &str = "I wanted to let you know that the report is finished and that I \
    will send it to the whole team before the meeting tomorrow morning";
const SHORT: &str = "send it to the team today";

fn request(
    settings: &AppSettings,
    available: bool,
    window: WindowContext,
) -> Option<PromptContext> {
    match cleanup_plan(settings, available, LONG, window) {
        CleanupPlan::Run(context) => Some(context),
        _ => None,
    }
}

/// Runs `text` through the output step with a fake cleanup that replies
/// `reply`, or fails without one. Returns the result and whether cleanup
/// was asked.
fn deliver_with(
    settings: &AppSettings,
    text: &str,
    language: Option<PassLanguage>,
    reply: Option<&str>,
) -> (ProcessedText, bool) {
    let asked = Cell::new(false);
    let plan = cleanup_plan(settings, true, text, WindowContext::default());
    let processed = tauri::async_runtime::block_on(deliver(
        text.to_string(),
        plan,
        language,
        |raw, _context| {
            asked.set(true);
            let processed = match reply {
                Some(reply) => ProcessedText {
                    text: reply.to_string(),
                    raw,
                    cleanup: Some(CleanupOutcome::Cleaned),
                },
                None => ProcessedText {
                    text: raw.clone(),
                    raw,
                    cleanup: Some(CleanupOutcome::Failed),
                },
            };
            future::ready(processed)
        },
    ));
    (processed, asked.get())
}

fn cleanup_on() -> AppSettings {
    let mut settings = get_default_settings();
    settings.ai_cleanup_enabled = true;
    settings
}

#[test]
fn short_dictation_skips_default_cleanup() {
    let (processed, asked) = deliver_with(&cleanup_on(), SHORT, None, Some("Cleaned."));
    assert!(!asked);
    assert_eq!(processed, ProcessedText::short_dictation(SHORT.to_string()));
}

#[test]
fn short_dictation_keeps_explicit_transform() {
    for style in [DictationStyle::Formal, DictationStyle::Casual] {
        let mut settings = cleanup_on();
        settings.dictation_style = style;
        let (processed, asked) = deliver_with(&settings, SHORT, None, Some("Cleaned."));
        assert!(asked, "{style:?}");
        assert_eq!(processed.cleanup, Some(CleanupOutcome::Cleaned));
    }
}

#[test]
fn long_dictation_still_cleaned() {
    assert!(LONG.unicode_words().count() >= SHORT_DICTATION_WORDS);
    let (processed, asked) = deliver_with(&cleanup_on(), LONG, None, Some("Cleaned."));
    assert!(asked);
    assert_eq!(processed.text, "Cleaned.");
}

#[test]
fn cjk_text_is_counted_by_words() {
    let long = "我们明天上午十点在会议室开会讨论新产品的发布计划和预算";
    assert_eq!(long.split_whitespace().count(), 1);
    let (_, asked) = deliver_with(&cleanup_on(), long, None, Some("Cleaned."));
    assert!(asked, "{} words", long.unicode_words().count());

    let short = "我们明天开会";
    let (processed, asked) = deliver_with(&cleanup_on(), short, None, Some("Cleaned."));
    assert!(!asked);
    assert_eq!(processed.cleanup, Some(CleanupOutcome::ShortDictation));
}

#[test]
fn setting_on_cleans_short_dictations() {
    let mut settings = cleanup_on();
    settings.cleanup_short_dictations = true;
    let (processed, asked) = deliver_with(&settings, SHORT, None, Some("Cleaned."));
    assert!(asked);
    assert_eq!(processed.cleanup, Some(CleanupOutcome::Cleaned));
}

#[test]
fn raw_fallback_gets_spoken_punctuation() {
    let english = Some(PassLanguage::En);
    let mut off = cleanup_on();
    off.ai_cleanup_enabled = false;
    let (processed, _) = deliver_with(&off, "hello comma world", english, None);
    assert_eq!(
        processed,
        ProcessedText::unprocessed("hello, world".to_string())
    );

    let (processed, _) = deliver_with(&cleanup_on(), "hello comma world", english, None);
    assert_eq!(
        processed,
        ProcessedText::short_dictation("hello, world".to_string())
    );

    let failed = format!("{LONG} comma thanks");
    let (processed, asked) = deliver_with(&cleanup_on(), &failed, english, None);
    assert!(asked);
    assert_eq!(processed.text, format!("{LONG}, thanks"));
    assert_eq!(processed.cleanup, Some(CleanupOutcome::Failed));
}

#[test]
fn cleaned_text_is_not_punctuated_twice() {
    let (processed, _) = deliver_with(
        &cleanup_on(),
        LONG,
        Some(PassLanguage::En),
        Some("Say comma twice"),
    );
    assert_eq!(processed.text, "Say comma twice");
}

#[test]
fn nothing_is_sent_with_cleanup_off() {
    let mut settings = get_default_settings();
    settings.window_names_enabled = true;
    settings.ai_cleanup_enabled = false;
    assert!(request(&settings, true, window()).is_none());
}

#[test]
fn nothing_is_sent_without_a_session() {
    let mut settings = get_default_settings();
    settings.window_names_enabled = true;
    settings.ai_cleanup_enabled = true;
    assert!(request(&settings, false, window()).is_none());
}

#[test]
fn a_setting_turned_off_while_dictating_drops_names_and_excerpt() {
    let mut settings = get_default_settings();
    settings.window_names_enabled = true;
    settings.ai_cleanup_enabled = true;
    settings.custom_words = vec!["Murmur".to_string()];
    let context = request(&settings, true, window()).expect("cleanup runs");
    assert_eq!(context.names, vec!["Ingrid"]);
    assert_eq!(context.field, window().field);

    settings.window_names_enabled = false;
    let context = request(&settings, true, window()).expect("cleanup runs");
    assert!(context.names.is_empty());
    assert_eq!(context.field, None);
    assert_eq!(context.vocabulary, vec!["Murmur"]);
}

#[test]
fn completed_operation_returns_its_output() {
    let result =
        tauri::async_runtime::block_on(complete_unless_cancelled(future::ready("done"), || false));

    assert_eq!(result, Some("done"));
}

#[test]
fn pending_operation_stops_after_cancellation() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_for_thread = Arc::clone(&cancelled);
    let cancel_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        cancelled_for_thread.store(true, Ordering::Release);
    });

    let result =
        tauri::async_runtime::block_on(complete_unless_cancelled(future::pending::<()>(), || {
            cancelled.load(Ordering::Acquire)
        }));

    cancel_thread.join().unwrap();
    assert_eq!(result, None);
}

#[test]
fn uncommitted_wav_is_removed_when_its_owner_drops() {
    let directory = tempfile::tempdir().unwrap();
    let wav_path = directory.path().join("pending.wav");

    drop(PendingWav::reserve(wav_path.clone()).unwrap());

    assert!(!wav_path.exists());
}

#[test]
fn wav_writer_keeps_shared_samples_alive_and_cleans_uncommitted_output() {
    let directory = tempfile::tempdir().unwrap();
    let wav_path = directory.path().join("shared-samples.wav");
    let samples = Arc::new(vec![0.25_f32, -0.5, 0.0]);

    let writer = PendingWav::reserve(wav_path.clone())
        .unwrap()
        .write(Arc::clone(&samples));
    drop(samples);

    let (wav, write_result) = tauri::async_runtime::block_on(writer).unwrap();
    write_result.unwrap();
    crate::audio_toolkit::verify_wav_file(&wav_path, 3).unwrap();

    drop(wav);
    assert!(!wav_path.exists());
}

#[test]
fn settle_wav_hands_back_a_verified_recording() {
    let directory = tempfile::tempdir().unwrap();
    let wav_path = directory.path().join("verified.wav");
    let writer = PendingWav::reserve(wav_path.clone())
        .unwrap()
        .write(Arc::new(vec![0.1_f32; 160]));

    let settled = tauri::async_runtime::block_on(settle_wav(Some(writer), &wav_path, 160));

    let mut wav = settled.expect("a complete file is kept");
    assert!(wav_path.exists());
    wav.commit();
}

#[test]
fn settle_wav_removes_a_file_that_fails_its_check() {
    let directory = tempfile::tempdir().unwrap();
    let wav_path = directory.path().join("short.wav");
    let writer = PendingWav::reserve(wav_path.clone())
        .unwrap()
        .write(Arc::new(vec![0.1_f32; 160]));

    let settled = tauri::async_runtime::block_on(settle_wav(Some(writer), &wav_path, 161));

    assert!(settled.is_none());
    assert!(!wav_path.exists());
}

#[test]
fn settle_wav_without_a_writer_has_nothing_to_keep() {
    let path = Path::new("never-written.wav");
    assert!(tauri::async_runtime::block_on(settle_wav(None, path, 0)).is_none());
}

#[test]
fn committed_wav_survives_its_owner() {
    let directory = tempfile::tempdir().unwrap();
    let wav_path = directory.path().join("committed.wav");

    let mut wav = PendingWav::reserve(wav_path.clone()).unwrap();
    fs::write(&wav_path, b"saved wav").unwrap();
    wav.commit();
    drop(wav);

    assert!(wav_path.exists());
}

#[test]
fn uncommitted_wav_is_removed_during_panic_unwind() {
    let directory = tempfile::tempdir().unwrap();
    let wav_path = directory.path().join("panic.wav");

    let result = std::panic::catch_unwind(|| {
        let _wav = PendingWav::reserve(wav_path.clone()).unwrap();
        panic!("simulated pipeline panic");
    });

    assert!(result.is_err());
    assert!(!wav_path.exists());
}

#[test]
fn reserving_a_referenced_wav_never_claims_or_deletes_it() {
    let directory = tempfile::tempdir().unwrap();
    let wav_path = directory.path().join("referenced.wav");
    fs::write(&wav_path, b"history-owned wav").unwrap();

    assert!(PendingWav::reserve(wav_path.clone()).is_err());

    assert_eq!(fs::read(&wav_path).unwrap(), b"history-owned wav");
}

#[test]
fn pasted_or_recoverable_output_survives_late_cancellation() {
    assert!(!paste_outcome_commits_history(PasteOutcome::Cancelled));
    assert!(paste_outcome_commits_history(PasteOutcome::Injected));
    assert!(paste_outcome_commits_history(PasteOutcome::CopyOffered));
    assert!(paste_outcome_commits_history(PasteOutcome::IntentionalNone));
    assert!(paste_outcome_commits_history(PasteOutcome::Skipped));
}

fn plan_for(settings: &AppSettings, text: &str, category: AppCategory) -> CleanupPlan {
    let started_in = WindowContext {
        kind: Some(category),
        ..WindowContext::default()
    };
    cleanup_plan(settings, true, text, started_in)
}

#[test]
fn a_dictation_reads_the_kind_of_app_only_when_a_kind_has_a_style() {
    let mut settings = get_default_settings();
    assert!(!context_reads(&settings).kind);

    settings
        .category_styles
        .insert(AppCategory::Emails, CategoryStyle::Formal);
    assert!(context_reads(&settings).kind);

    // The window names switch does not gate it: the kind is not window text.
    settings.window_names_enabled = false;
    assert!(context_reads(&settings).kind);

    settings.ai_cleanup_enabled = false;
    assert!(!context_reads(&settings).kind);
}

#[test]
fn a_kind_of_app_with_its_own_style_does_not_follow_the_general_one() {
    let mut settings = get_default_settings();
    settings.dictation_style = DictationStyle::Formal;
    settings
        .category_styles
        .insert(AppCategory::PersonalMessages, CategoryStyle::Casual);

    let style = |category| match plan_for(&settings, LONG, category) {
        CleanupPlan::Run(context) => Some(context.style),
        _ => None,
    };
    assert_eq!(
        style(AppCategory::PersonalMessages),
        Some(DictationStyle::Casual)
    );
    assert_eq!(style(AppCategory::Emails), Some(DictationStyle::Formal));
}

#[test]
fn a_kind_of_app_set_to_off_gets_no_cleanup() {
    let mut settings = get_default_settings();
    settings
        .category_styles
        .insert(AppCategory::Coding, CategoryStyle::Off);

    assert!(matches!(
        plan_for(&settings, LONG, AppCategory::Coding),
        CleanupPlan::Off
    ));
    assert!(matches!(
        plan_for(&settings, LONG, AppCategory::Emails),
        CleanupPlan::Run(_)
    ));
}

#[test]
fn a_short_dictation_is_cleaned_only_where_the_style_asks_for_it() {
    let mut settings = get_default_settings();
    settings
        .category_styles
        .insert(AppCategory::Emails, CategoryStyle::Formal);
    assert!(matches!(
        plan_for(&settings, SHORT, AppCategory::Emails),
        CleanupPlan::Run(_)
    ));
    assert!(matches!(
        plan_for(&settings, SHORT, AppCategory::Documents),
        CleanupPlan::Short { .. }
    ));

    // Normal for one kind wins over a general Formal: the short text is left alone.
    settings.dictation_style = DictationStyle::Formal;
    settings
        .category_styles
        .insert(AppCategory::WorkMessages, CategoryStyle::Normal);
    assert!(matches!(
        plan_for(&settings, SHORT, AppCategory::WorkMessages),
        CleanupPlan::Short { .. }
    ));
}

#[test]
fn settings_written_before_styles_by_kind_have_none() {
    let old: AppSettings = serde_json::from_value(
        serde_json::to_value(get_default_settings())
            .map(|mut value| {
                value.as_object_mut().unwrap().remove("category_styles");
                value
            })
            .unwrap(),
    )
    .unwrap();
    assert!(old.category_styles.is_empty());

    let mut settings = get_default_settings();
    settings
        .category_styles
        .insert(AppCategory::AiPrompts, CategoryStyle::Off);
    let json = serde_json::to_value(&settings).unwrap();
    assert_eq!(json["category_styles"]["ai_prompts"], "off");
}
