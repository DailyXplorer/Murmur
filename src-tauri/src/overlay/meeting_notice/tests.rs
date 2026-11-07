use super::*;
use crate::meetings::announce::MeetingName;

const IDLE: Context = Context {
    enabled: true,
    dictation: false,
    viewed_meeting: None,
};

fn finished(id: i64, outcome: Outcome) -> Finished {
    Finished { id, outcome }
}

fn batch(notices: &Notices) -> &Batch {
    match &notices.current {
        Some(Notice::MeetingNews(news)) => &news.batch,
        _ => panic!("No meeting announcement is showing"),
    }
}

fn listening() -> Notices {
    Notices {
        meeting_listener_ready: true,
        ..Notices::default()
    }
}

fn ready_current_page(notices: &mut Notices, context: Context) -> bool {
    let epoch = notices.page_epoch;
    page_ready(notices, context, epoch)
}

#[test]
fn a_ready_page_does_not_show_the_same_news_twice() {
    let mut notices = Notices::default();
    receive(&mut notices, IDLE, finished(1, Outcome::Ready));

    assert!(ready_current_page(&mut notices, IDLE));
    assert!(!ready_current_page(&mut notices, IDLE));
    assert_eq!(batch(&notices).single(), Some(finished(1, Outcome::Ready)));
    assert!(notices.pending_meetings.is_none());
}

#[test]
fn a_retired_pages_ready_cannot_release_the_replacement_pages_news() {
    let mut notices = Notices::default();
    let retired_epoch = notices.page_epoch;
    let ready = finished(1, Outcome::Ready);
    assert_eq!(receive(&mut notices, IDLE, ready), Reception::Waiting);
    assert!(!page_loading(&mut notices));

    assert!(!page_ready(&mut notices, IDLE, retired_epoch));
    assert!(!notices.meeting_listener_ready);
    assert!(notices.current.is_none());
    assert_eq!(
        notices.pending_meetings.as_ref().and_then(Batch::single),
        Some(ready)
    );

    assert!(ready_current_page(&mut notices, IDLE));
    assert_eq!(batch(&notices).single(), Some(ready));
    assert!(notices.pending_meetings.is_none());
    assert!(!ready_current_page(&mut notices, IDLE));
}

#[test]
fn every_page_reset_rejects_the_previous_epoch_even_without_showing_news() {
    let mut notices = Notices::default();
    let generation = AtomicU64::new(0);
    for _ in 0..3 {
        let retired_epoch = notices.page_epoch;
        assert!(!reset_page_state(&mut notices, &generation));
        assert!(!page_ready(&mut notices, IDLE, retired_epoch));
        assert!(!notices.meeting_listener_ready);
        assert!(!ready_current_page(&mut notices, IDLE));
        assert!(notices.meeting_listener_ready);
    }
    assert_eq!(generation.load(Ordering::SeqCst), 0);
}

#[test]
fn a_page_reload_replays_showing_news_with_a_fresh_deadline() {
    let mut notices = listening();
    let retired_epoch = notices.page_epoch;
    let generation = AtomicU64::new(0);
    let ready = finished(1, Outcome::Ready);
    assert_eq!(receive(&mut notices, IDLE, ready), Reception::Show);
    let old_deadline = generation.fetch_add(1, Ordering::SeqCst) + 1;

    assert!(reset_page_state(&mut notices, &generation));
    assert!(notices.current.is_none());
    assert_eq!(
        notices.pending_meetings.as_ref().and_then(Batch::single),
        Some(ready)
    );
    assert_eq!(
        dismiss(
            &mut notices.current,
            Dismissal::Timer(old_deadline),
            generation.load(Ordering::SeqCst),
            OverlayStyle::Minimal
        ),
        None
    );
    assert!(!reset_page_state(&mut notices, &generation));
    assert!(!resume(&mut notices, IDLE));
    assert!(!page_ready(&mut notices, IDLE, retired_epoch));

    assert!(ready_current_page(&mut notices, IDLE));
    let fresh_deadline = generation.fetch_add(1, Ordering::SeqCst) + 1;
    assert_eq!(batch(&notices).single(), Some(ready));
    assert!(notices.pending_meetings.is_none());
    assert_eq!(
        dismiss(
            &mut notices.current,
            Dismissal::Timer(old_deadline),
            generation.load(Ordering::SeqCst),
            OverlayStyle::Minimal
        ),
        None
    );
    assert_eq!(batch(&notices).single(), Some(ready));
    assert_eq!(
        dismiss(
            &mut notices.current,
            Dismissal::Timer(fresh_deadline),
            generation.load(Ordering::SeqCst),
            OverlayStyle::Minimal
        ),
        Some(AfterDismissal::Settled)
    );
}

#[test]
fn completions_during_a_page_reload_wait_until_its_listener_is_ready() {
    let mut notices = listening();
    assert!(!page_loading(&mut notices));
    assert_eq!(
        receive(&mut notices, IDLE, finished(1, Outcome::Ready)),
        Reception::Waiting
    );
    assert_eq!(
        receive(&mut notices, IDLE, finished(2, Outcome::Failed)),
        Reception::Waiting
    );
    assert!(notices.current.is_none());
    assert!(!resume(&mut notices, IDLE));

    assert!(ready_current_page(&mut notices, IDLE));
    assert_eq!(
        batch(&notices).content(|_| None),
        Some(Content::Batch {
            ready: 1,
            failed: 1
        })
    );
    assert!(notices.pending_meetings.is_none());
}

#[test]
fn a_dictation_during_a_page_reload_preserves_suspended_and_new_news() {
    let busy = Context {
        dictation: true,
        ..IDLE
    };
    for showing in [false, true] {
        let mut notices = listening();
        let ready = finished(1, Outcome::Ready);
        if showing {
            assert_eq!(receive(&mut notices, IDLE, ready), Reception::Show);
        }
        assert_eq!(page_loading(&mut notices), showing);
        if !showing {
            assert_eq!(receive(&mut notices, IDLE, ready), Reception::Waiting);
        }
        assert_eq!(
            receive(&mut notices, IDLE, finished(2, Outcome::Failed)),
            Reception::Waiting
        );
        assert!(dictation_takes_overlay(&mut notices.current, "recording"));
        assert!(!ready_current_page(&mut notices, busy));
        assert!(!resume(&mut notices, busy));
        assert!(notices.current.is_none());

        assert!(resume(&mut notices, IDLE));
        assert_eq!(
            batch(&notices).content(|_| None),
            Some(Content::Batch {
                ready: 1,
                failed: 1
            })
        );
    }
}

#[test]
fn another_notice_cannot_release_pending_news_during_a_page_reload() {
    let mut notices = Notices {
        current: Some(Notice::Error),
        ..listening()
    };
    receive(&mut notices, IDLE, finished(1, Outcome::Ready));
    assert!(!page_loading(&mut notices));
    assert_eq!(
        dismiss(
            &mut notices.current,
            Dismissal::Timer(1),
            1,
            OverlayStyle::Minimal
        ),
        Some(AfterDismissal::Settled)
    );
    assert!(!resume(&mut notices, IDLE));
    assert!(ready_current_page(&mut notices, IDLE));
    assert_eq!(batch(&notices).single(), Some(finished(1, Outcome::Ready)));
}

#[test]
fn completions_wait_for_the_page_listener_and_replay_without_losing_the_batch() {
    let mut notices = Notices::default();
    assert_eq!(
        receive(&mut notices, IDLE, finished(1, Outcome::Ready)),
        Reception::Waiting
    );
    assert_eq!(
        receive(&mut notices, IDLE, finished(2, Outcome::Failed)),
        Reception::Waiting
    );
    assert!(notices.current.is_none());
    assert!(!resume(&mut notices, IDLE));

    assert!(ready_current_page(&mut notices, IDLE));
    let expected = Content::Batch {
        ready: 1,
        failed: 1,
    };
    assert_eq!(batch(&notices).content(|_| None), Some(expected.clone()));
    assert!(notices.pending_meetings.is_none());
    assert!(!ready_current_page(&mut notices, IDLE));
    assert_eq!(batch(&notices).content(|_| None), Some(expected));
}

#[test]
fn page_readiness_leaves_dictation_and_other_notices_ahead_of_waiting_news() {
    let busy = Context {
        dictation: true,
        ..IDLE
    };
    let mut notices = Notices::default();
    receive(&mut notices, busy, finished(1, Outcome::Ready));
    assert!(!ready_current_page(&mut notices, busy));
    assert!(notices.current.is_none());
    notices.current = Some(Notice::Error);
    assert!(!ready_current_page(&mut notices, IDLE));
    assert!(matches!(notices.current, Some(Notice::Error)));
    notices.current = None;
    assert!(resume(&mut notices, IDLE));
    assert_eq!(batch(&notices).single(), Some(finished(1, Outcome::Ready)));
}

#[test]
fn page_readiness_drops_news_that_was_disabled_or_viewed_while_loading() {
    for context in [
        Context {
            enabled: false,
            ..IDLE
        },
        Context {
            viewed_meeting: Some(7),
            ..IDLE
        },
    ] {
        let mut notices = Notices::default();
        receive(&mut notices, IDLE, finished(7, Outcome::Ready));
        assert!(!ready_current_page(&mut notices, context));
        assert!(notices.current.is_none());
        assert!(notices.pending_meetings.is_none());
    }
}

#[test]
fn a_finished_meeting_shows_once_and_new_outcomes_merge_in_place() {
    let mut notices = listening();
    let ready = finished(1, Outcome::Ready);
    assert_eq!(receive(&mut notices, IDLE, ready), Reception::Show);
    assert_eq!(receive(&mut notices, IDLE, ready), Reception::Ignored);
    assert_eq!(
        receive(&mut notices, IDLE, finished(2, Outcome::Failed)),
        Reception::Update
    );
    assert_eq!(
        batch(&notices).content(|_| None),
        Some(Content::Batch {
            ready: 1,
            failed: 1
        })
    );
    assert!(notices.pending_meetings.is_none());
    assert_eq!(
        dismiss(
            &mut notices.current,
            Dismissal::Escape,
            0,
            OverlayStyle::Minimal
        ),
        Some(AfterDismissal::Settled)
    );
    assert_eq!(
        receive(&mut notices, IDLE, finished(3, Outcome::Ready)),
        Reception::Show
    );
    assert_eq!(
        batch(&notices).open_target(),
        crate::meetings::announce::OpenTarget::Meeting(3)
    );
}

#[test]
fn outcomes_merge_while_waiting_for_a_dictation_then_show_together() {
    let mut notices = listening();
    let busy = Context {
        dictation: true,
        ..IDLE
    };
    assert_eq!(
        receive(&mut notices, busy, finished(1, Outcome::Ready)),
        Reception::Waiting
    );
    assert_eq!(
        receive(&mut notices, busy, finished(2, Outcome::Failed)),
        Reception::Waiting
    );
    assert!(notices.current.is_none());
    assert!(!resume(&mut notices, busy));
    assert!(resume(&mut notices, IDLE));
    assert_eq!(
        batch(&notices).content(|_| None),
        Some(Content::Batch {
            ready: 1,
            failed: 1
        })
    );
    assert!(notices.pending_meetings.is_none());
}

#[test]
fn other_notices_and_a_chained_microphone_warning_finish_before_the_meeting_news() {
    for current in [
        Some(Notice::Error),
        Some(Notice::MicLost),
        Some(Notice::Cancelled),
        Some(Notice::Copy {
            text: "dictation".to_string(),
            write: Arc::new(|_| Ok(())),
            mic_lost_next: true,
        }),
    ] {
        let mut notices = Notices {
            current,
            ..listening()
        };
        assert_eq!(
            receive(&mut notices, IDLE, finished(1, Outcome::Ready)),
            Reception::Waiting
        );
        assert!(!resume(&mut notices, IDLE));
        let next = dismiss(
            &mut notices.current,
            Dismissal::Timer(0),
            0,
            OverlayStyle::Minimal,
        );
        if next == Some(AfterDismissal::Warning) {
            assert!(!resume(&mut notices, IDLE));
            dismiss(
                &mut notices.current,
                Dismissal::Timer(1),
                1,
                OverlayStyle::Minimal,
            );
        }
        assert!(resume(&mut notices, IDLE));
        assert_eq!(batch(&notices).single(), Some(finished(1, Outcome::Ready)));
    }
}

#[test]
fn a_new_dictation_drops_showing_news_but_not_a_waiting_batch() {
    let mut notices = listening();
    receive(&mut notices, IDLE, finished(1, Outcome::Ready));
    assert!(dictation_takes_overlay(&mut notices.current, "recording"));
    assert!(!resume(&mut notices, IDLE));
    receive(
        &mut notices,
        Context {
            dictation: true,
            ..IDLE
        },
        finished(2, Outcome::Ready),
    );
    assert!(dictation_takes_overlay(
        &mut notices.current,
        "transcribing"
    ));
    assert!(resume(&mut notices, IDLE));
    assert_eq!(batch(&notices).single(), Some(finished(2, Outcome::Ready)));
}

#[test]
fn restoring_a_dictation_takes_the_overlay_from_showing_meeting_news() {
    let mut notices = listening();
    receive(&mut notices, IDLE, finished(1, Outcome::Ready));
    assert!(dictation_takes_overlay(
        &mut notices.current,
        "transcribing"
    ));
    assert!(notices.current.is_none());
    assert!(!resume(&mut notices, IDLE));
}

#[test]
fn meeting_news_is_fitted_and_escape_returns_the_recording_meeting_pill() {
    let mut notices = listening();
    receive(&mut notices, IDLE, finished(1, Outcome::Ready));
    assert!(notices.current.as_ref().is_some_and(Notice::is_fitted));
    assert_eq!(
        dismiss(
            &mut notices.current,
            Dismissal::Escape,
            0,
            OverlayStyle::Minimal
        ),
        Some(AfterDismissal::Settled)
    );
    assert!(!resume(&mut notices, IDLE));
    let recording = MeetingPill {
        started_at_ms: 1,
        stops_at_ms: 2,
    };
    assert_eq!(
        resting_pill(Some(recording), OverlayStyle::Minimal),
        Some(recording)
    );
    assert_eq!(resting_pill(None, OverlayStyle::Minimal), None);
}

#[test]
fn disabled_overlays_and_the_meeting_already_visible_in_settings_drop_the_outcome() {
    for context in [
        Context {
            enabled: false,
            ..IDLE
        },
        Context {
            viewed_meeting: Some(7),
            ..IDLE
        },
    ] {
        let mut notices = listening();
        assert_eq!(
            receive(&mut notices, context, finished(7, Outcome::Ready)),
            Reception::Ignored
        );
        assert!(notices.current.is_none());
        assert!(notices.pending_meetings.is_none());
    }
    let mut notices = listening();
    assert_eq!(
        receive(
            &mut notices,
            Context {
                viewed_meeting: Some(8),
                ..IDLE
            },
            finished(7, Outcome::Ready)
        ),
        Reception::Show
    );
}

#[test]
fn opening_a_meeting_removes_its_pending_or_showing_outcome() {
    let mut notices = listening();
    receive(
        &mut notices,
        Context {
            dictation: true,
            ..IDLE
        },
        finished(7, Outcome::Ready),
    );
    assert!(!resume(
        &mut notices,
        Context {
            viewed_meeting: Some(7),
            ..IDLE
        }
    ));
    assert!(notices.pending_meetings.is_none());
    receive(&mut notices, IDLE, finished(7, Outcome::Ready));
    receive(&mut notices, IDLE, finished(8, Outcome::Failed));
    assert!(forget_viewed(&mut notices, Some(7)));
    assert_eq!(batch(&notices).single(), Some(finished(8, Outcome::Failed)));
    assert!(forget_viewed(&mut notices, Some(8)));
    assert!(notices.current.is_none());
}

#[test]
fn pending_news_reads_the_title_written_while_it_waited() {
    let mut notices = listening();
    receive(
        &mut notices,
        Context {
            dictation: true,
            ..IDLE
        },
        finished(7, Outcome::Ready),
    );
    let name = MeetingName {
        title: Some("Planning".to_string()),
        started_at_ms: 123,
    };
    assert!(resume(&mut notices, IDLE));
    let content = batch(&notices).content(|_| Some(name)).unwrap();
    let labels = labels(content, &get_tray_translations(Some("en".into())));
    assert_eq!(labels.headline, "Planning");
    assert_eq!(labels.detail.as_deref(), Some("Transcript ready"));
}

#[test]
fn native_labels_cover_ready_failed_and_mixed_batches_in_all_locales() {
    for (locale, ready, failed, mixed) in [
        (
            "en",
            "2 meetings ready",
            "Couldn't transcribe 2 meetings",
            "1 couldn't be transcribed",
        ),
        (
            "fr",
            "2 réunions prêtes",
            "Impossible de transcrire 2 réunions",
            "1 n'a pas pu être transcrite",
        ),
        (
            "de",
            "2 Meetings bereit",
            "2 Meetings konnten nicht transkribiert werden",
            "1 konnte nicht transkribiert werden",
        ),
        (
            "es",
            "2 reuniones listas",
            "No se pudieron transcribir 2 reuniones",
            "No se pudo transcribir 1",
        ),
        (
            "it",
            "2 riunioni pronte",
            "Impossibile trascrivere 2 riunioni",
            "1 non è stata trascritta",
        ),
        (
            "pt",
            "2 reuniões prontas",
            "Não foi possível transcrever 2 reuniões",
            "Não foi possível transcrever 1",
        ),
    ] {
        let strings = get_tray_translations(Some(locale.into()));
        let only_ready = labels(
            Content::Batch {
                ready: 2,
                failed: 0,
            },
            &strings,
        );
        assert_eq!(only_ready.headline, ready);
        assert_eq!(only_ready.detail, None);
        assert_eq!(only_ready.tone, Tone::Ready);
        let only_failed = labels(
            Content::Batch {
                ready: 0,
                failed: 2,
            },
            &strings,
        );
        assert_eq!(only_failed.headline, failed);
        assert_eq!(only_failed.tone, Tone::Failed);
        let mixed_labels = labels(
            Content::Batch {
                ready: 2,
                failed: 1,
            },
            &strings,
        );
        assert_eq!(mixed_labels.headline, ready);
        assert_eq!(mixed_labels.detail.as_deref(), Some(mixed));
        assert_eq!(mixed_labels.tone, Tone::Ready);
        for labels in [only_ready, only_failed, mixed_labels] {
            assert_eq!(labels.meeting_id, None);
            assert_eq!(labels.copy, None);
            assert_eq!(labels.default_name, None);
        }
    }
}

#[test]
fn an_untitled_meeting_sends_the_lists_template_date_and_resolved_locale() {
    let strings = get_tray_translations(Some("fr-FR".into()));
    for outcome in [Outcome::Ready, Outcome::Failed] {
        let labels = labels(
            Content::One {
                finished: finished(7, outcome),
                name: MeetingName {
                    title: None,
                    started_at_ms: 123,
                },
            },
            &strings,
        );
        assert_eq!(
            labels.default_name,
            Some(DefaultName {
                started_at_ms: 123,
                locale: "fr".to_string()
            })
        );
        let template = if outcome == Outcome::Ready {
            Some(labels.headline.as_str())
        } else {
            labels.detail.as_deref()
        };
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../src/i18n/locales/fr/translation.json"
        ))
        .unwrap();
        assert_eq!(template, source["meetings"]["list"]["untitled"].as_str());
        assert_eq!(labels.copy.is_some(), outcome == Outcome::Ready);
        assert_eq!(labels.meeting_id, Some(7));
    }
}
