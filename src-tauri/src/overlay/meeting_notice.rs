use super::*;
use crate::meetings::announce::{Batch, Content, Finished, Outcome};
use crate::meetings::MeetingsManager;
use crate::settings_window::{self, MainAction};
use crate::tray_i18n::TrayStrings;

const DISPLAY_TIME: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct DefaultName {
    started_at_ms: i64,
    locale: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Tone {
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct MeetingNoticeLabels {
    meeting_id: Option<i64>,
    tone: Tone,
    headline: String,
    detail: Option<String>,
    open: String,
    copy: Option<String>,
    copied: String,
    close: String,
    default_name: Option<DefaultName>,
}

pub(super) struct MeetingNews {
    batch: Batch,
    labels: Option<MeetingNoticeLabels>,
}

impl MeetingNews {
    fn new(batch: Batch) -> Self {
        Self {
            batch,
            labels: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Context {
    enabled: bool,
    dictation: bool,
    viewed_meeting: Option<i64>,
}

impl Context {
    fn current(app: &AppHandle, enabled: bool) -> Self {
        Self {
            enabled,
            dictation: DICTATION_SHOWING.load(Ordering::SeqCst),
            viewed_meeting: settings_window::viewed_meeting(app),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Reception {
    Ignored,
    Waiting,
    Show,
    Update,
}

fn receive(notices: &mut Notices, context: Context, finished: Finished) -> Reception {
    if !context.enabled || context.viewed_meeting == Some(finished.id) {
        return Reception::Ignored;
    }
    if let Some(Notice::MeetingNews(news)) = &mut notices.current {
        return if news.batch.merge(finished) {
            Reception::Update
        } else {
            Reception::Ignored
        };
    }
    if notices.current.is_some() || context.dictation {
        if let Some(batch) = &mut notices.pending_meetings {
            batch.merge(finished);
        } else {
            notices.pending_meetings = Some(Batch::new(finished));
        }
        return Reception::Waiting;
    }
    let batch = match notices.pending_meetings.take() {
        Some(mut batch) => {
            batch.merge(finished);
            batch
        }
        None => Batch::new(finished),
    };
    notices.current = Some(Notice::MeetingNews(MeetingNews::new(batch)));
    Reception::Show
}

fn resume(notices: &mut Notices, context: Context) -> bool {
    if !context.enabled {
        notices.pending_meetings = None;
        return false;
    }
    if notices.current.is_some() || context.dictation {
        return false;
    }
    forget_viewed(notices, context.viewed_meeting);
    let Some(batch) = notices.pending_meetings.take() else {
        return false;
    };
    notices.current = Some(Notice::MeetingNews(MeetingNews::new(batch)));
    true
}

/// Also removes outcomes queued before the user opened their meeting.
fn forget_viewed(notices: &mut Notices, viewed: Option<i64>) -> bool {
    let Some(id) = viewed else { return false };
    notices.pending_meetings = notices
        .pending_meetings
        .as_ref()
        .and_then(|batch| batch.without(id));
    let Some(Notice::MeetingNews(news)) = &mut notices.current else {
        return false;
    };
    let remaining = news.batch.without(id);
    if remaining.as_ref() == Some(&news.batch) {
        return false;
    }
    if let Some(batch) = remaining {
        news.batch = batch;
    } else {
        notices.current = None;
    }
    true
}

fn counted(one: &str, other: &str, count: usize) -> String {
    if count == 1 { one } else { other }.replace("{{count}}", &count.to_string())
}

fn labels(content: Content, strings: &TrayStrings) -> MeetingNoticeLabels {
    let meeting_id = match &content {
        Content::One { finished, .. } => Some(finished.id),
        Content::Batch { .. } => None,
    };
    let (tone, headline, detail, copy, default_name) = match content {
        Content::One { finished, name } => {
            let default_name = name.title.is_none().then(|| DefaultName {
                started_at_ms: name.started_at_ms,
                locale: strings.locale.to_string(),
            });
            let name = name
                .title
                .unwrap_or_else(|| strings.overlay_meeting_untitled.clone());
            match finished.outcome {
                Outcome::Ready => (
                    Tone::Ready,
                    name,
                    Some(strings.overlay_transcript_ready.clone()),
                    Some(strings.overlay_copy.clone()),
                    default_name,
                ),
                Outcome::Failed => (
                    Tone::Failed,
                    strings.overlay_meeting_failed.clone(),
                    Some(name),
                    None,
                    default_name,
                ),
            }
        }
        Content::Batch { ready, failed } if ready > 0 => (
            Tone::Ready,
            counted(
                &strings.overlay_meetings_ready_one,
                &strings.overlay_meetings_ready_other,
                ready,
            ),
            (failed > 0).then(|| {
                counted(
                    &strings.overlay_meetings_untranscribed_one,
                    &strings.overlay_meetings_untranscribed_other,
                    failed,
                )
            }),
            None,
            None,
        ),
        Content::Batch { failed, .. } => (
            Tone::Failed,
            counted(
                &strings.overlay_meetings_failed_one,
                &strings.overlay_meetings_failed_other,
                failed,
            ),
            None,
            None,
            None,
        ),
    };
    MeetingNoticeLabels {
        meeting_id,
        tone,
        headline,
        detail,
        copy,
        default_name,
        open: strings.overlay_open.clone(),
        copied: strings.overlay_copied.clone(),
        close: strings.overlay_close.clone(),
    }
}

enum Presentation {
    Show,
    Merge,
    Refresh,
}

fn present(app: &AppHandle, notices: &mut Notices, presentation: Presentation) -> bool {
    let Some(Notice::MeetingNews(news)) = &mut notices.current else {
        return false;
    };
    let content = news.batch.content(|id| {
        let manager = app.try_state::<Arc<MeetingsManager>>()?;
        match manager.announcement_name(id) {
            Ok(name) => name,
            Err(error) => {
                log::warn!("Cannot read the announced meeting's name: {error}");
                None
            }
        }
    });
    let Some(content) = content else {
        notices.current = None;
        return false;
    };
    let strings = get_tray_translations(Some(settings::get_settings(app).app_language));
    let labels = labels(content, &strings);
    let new = matches!(presentation, Presentation::Show);
    if new
        && present_overlay_window(
            app,
            "meeting-news",
            COPY_OVERLAY_WIDTH,
            COPY_OVERLAY_HEIGHT,
            true,
        )
        .is_none()
    {
        notices.current = None;
        return false;
    }
    if new || news.labels.as_ref() != Some(&labels) {
        let _ = app.emit_to("recording_overlay", "show-overlay-meeting-notice", &labels);
        news.labels = Some(labels);
    }
    if !matches!(presentation, Presentation::Refresh) {
        let generation = OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        dismiss_notice_after(app, generation, DISPLAY_TIME, true);
    }
    true
}

pub(super) fn show_pending(app: &AppHandle, notices: &mut Notices, style: OverlayStyle) -> bool {
    let context = Context::current(app, style != OverlayStyle::None);
    resume(notices, context) && present(app, notices, Presentation::Show)
}

pub fn announce_meeting(app: &AppHandle, finished: Finished) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let mut held = notice();
        let enabled = settings::get_settings(&handle).overlay_style != OverlayStyle::None;
        let context = Context::current(&handle, enabled);
        match receive(&mut held, context, finished) {
            Reception::Show => {
                present(&handle, &mut held, Presentation::Show);
            }
            Reception::Update => {
                present(&handle, &mut held, Presentation::Merge);
            }
            Reception::Ignored | Reception::Waiting => {}
        }
    });
}

pub fn meeting_updated(app: &AppHandle, id: i64) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let mut held = notice();
        if matches!(&held.current, Some(Notice::MeetingNews(news)) if news.batch.single().is_some_and(|finished| finished.id == id))
            && !present(&handle, &mut held, Presentation::Refresh)
        {
            drop(held);
            settle_overlay(&handle);
        }
    });
}

pub fn settings_meeting_changed(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let mut held = notice();
        if !forget_viewed(&mut held, settings_window::viewed_meeting(&handle)) {
            return;
        }
        if present(&handle, &mut held, Presentation::Refresh) {
            return;
        }
        OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
        drop(held);
        settle_overlay(&handle);
    });
}

#[tauri::command]
#[specta::specta]
pub fn dismiss_meeting_notice(app: AppHandle) {
    dismiss_notice(&app, Dismissal::Meeting);
}

#[tauri::command]
#[specta::specta]
pub fn open_meeting_notice(app: AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let target = {
            let mut held = notice();
            let Some(Notice::MeetingNews(news)) = &held.current else {
                return;
            };
            let target = news.batch.open_target();
            held.current = None;
            OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst);
            target
        };
        settle_overlay(&handle);
        settings_window::queue(&handle, MainAction::OpenMeetings(target));
        crate::show_main_window(&handle);
        let _ = handle.emit_to(settings_window::MAIN_WINDOW, "open-meetings", ());
    });
}

#[tauri::command]
#[specta::specta]
pub async fn copy_meeting_notice(app: AppHandle) -> Result<(), String> {
    let (id, kind, generation) = {
        let held = notice();
        let target = match &held.current {
            Some(Notice::MeetingNews(news)) => news.batch.copy_target(),
            _ => None,
        }
        .ok_or_else(|| "There is no single ready meeting to copy".to_string())?;
        (
            target.0,
            target.1,
            OVERLAY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1,
        )
    };
    dismiss_notice_after(&app, generation, DISPLAY_TIME, true);
    let handle = app.clone();
    let markdown = tauri::async_runtime::spawn_blocking(move || {
        let manager = handle
            .try_state::<Arc<MeetingsManager>>()
            .ok_or_else(|| "Meetings are unavailable".to_string())?;
        manager.markdown(id, kind).map(|(markdown, _)| markdown)
    })
    .await
    .map_err(|error| format!("Meeting task failed: {error}"))??;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let held = notice();
        let still_showing = matches!(&held.current, Some(Notice::MeetingNews(news)) if news.batch.copy_target() == Some((id, kind)))
            && OVERLAY_GENERATION.load(Ordering::SeqCst) == generation;
        let result = if still_showing {
            crate::clipboard::copy_to_keep(&handle, &markdown)
        } else {
            Err("The meeting notice is no longer showing".to_string())
        };
        let _ = sender.send(result);
    }).map_err(|error| format!("Cannot reach the clipboard: {error}"))?;
    receiver
        .await
        .map_err(|_| "The clipboard write did not run".to_string())?
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_finished_meeting_shows_once_and_new_outcomes_merge_in_place() {
        let mut notices = Notices::default();
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
                Dismissal::Meeting,
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
        let mut notices = Notices::default();
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
                ..Notices::default()
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
        let mut notices = Notices::default();
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
        let mut notices = Notices::default();
        receive(&mut notices, IDLE, finished(1, Outcome::Ready));
        assert!(dictation_takes_overlay(&mut notices.current, "transcribing"));
        assert!(notices.current.is_none());
        assert!(!resume(&mut notices, IDLE));
    }

    #[test]
    fn meeting_news_is_fitted_and_escape_returns_the_recording_meeting_pill() {
        let mut notices = Notices::default();
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
        assert_eq!(
            dismiss(
                &mut notices.current,
                Dismissal::Meeting,
                1,
                OverlayStyle::Minimal
            ),
            None
        );
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
            let mut notices = Notices::default();
            assert_eq!(
                receive(&mut notices, context, finished(7, Outcome::Ready)),
                Reception::Ignored
            );
            assert!(notices.current.is_none());
            assert!(notices.pending_meetings.is_none());
        }
        let mut notices = Notices::default();
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
        let mut notices = Notices::default();
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
        let mut notices = Notices::default();
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
                "../../../src/i18n/locales/fr/translation.json"
            ))
            .unwrap();
            assert_eq!(template, source["meetings"]["list"]["untitled"].as_str());
            assert_eq!(labels.copy.is_some(), outcome == Outcome::Ready);
            assert_eq!(labels.meeting_id, Some(7));
        }
    }
}
