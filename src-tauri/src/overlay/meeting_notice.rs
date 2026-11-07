use super::*;
use crate::meetings::announce::{Batch, Content, Finished, Outcome};
use crate::meetings::MeetingsManager;
use crate::settings_window::{self, MainAction};
use crate::tray_i18n::TrayStrings;
use std::sync::Once;

const DISPLAY_TIME: Duration = Duration::from_secs(15);
static PAGE_LISTENER: Once = Once::new();

#[derive(Clone, serde::Serialize)]
struct PageEpochReply {
    nonce: String,
    epoch: String,
}

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
    if notices.current.is_some() || context.dictation || !notices.meeting_listener_ready {
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
    if notices.current.is_some() || context.dictation || !notices.meeting_listener_ready {
        return false;
    }
    forget_viewed(notices, context.viewed_meeting);
    let Some(batch) = notices.pending_meetings.take() else {
        return false;
    };
    notices.current = Some(Notice::MeetingNews(MeetingNews::new(batch)));
    true
}

fn page_loading(notices: &mut Notices) -> bool {
    notices.page_epoch += 1;
    notices.meeting_listener_ready = false;
    match notices.current.take() {
        Some(Notice::MeetingNews(news)) => {
            notices.pending_meetings = Some(news.batch);
            true
        }
        current => {
            notices.current = current;
            false
        }
    }
}

fn page_ready(notices: &mut Notices, context: Context, epoch: u64) -> bool {
    if epoch != notices.page_epoch || notices.meeting_listener_ready {
        return false;
    }
    notices.meeting_listener_ready = true;
    forget_viewed(notices, context.viewed_meeting);
    resume(notices, context)
}

fn reset_page_state(notices: &mut Notices, generation: &AtomicU64) -> bool {
    let suspended = page_loading(notices);
    if suspended {
        generation.fetch_add(1, Ordering::SeqCst);
    }
    suspended
}

pub(super) fn reset_page(app: &AppHandle) {
    let mut held = notice();
    if !reset_page_state(&mut held, &OVERLAY_GENERATION) {
        return;
    }
    drop(held);
    settle_overlay(app);
}

pub(super) fn listen_for_page(app: &AppHandle) {
    use tauri::Listener;

    PAGE_LISTENER.call_once(|| {
        let handle = app.clone();
        app.listen("meeting-notice-page-request", move |event| {
            let Ok(nonce) = serde_json::from_str::<String>(event.payload()) else {
                return;
            };
            let app = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let reply = PageEpochReply {
                    nonce,
                    epoch: notice().page_epoch.to_string(),
                };
                let _ = app.emit_to("recording_overlay", "meeting-notice-page-epoch", reply);
            });
        });

        let handle = app.clone();
        app.listen("meeting-notice-listener-ready", move |event| {
            let Some(epoch) = serde_json::from_str::<String>(event.payload())
                .ok()
                .and_then(|epoch| epoch.parse::<u64>().ok())
            else {
                return;
            };
            let app = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let mut held = notice();
                let enabled = settings::get_settings(&app).overlay_style != OverlayStyle::None;
                let context = Context::current(&app, enabled);
                if page_ready(&mut held, context, epoch)
                    && present(&app, &mut held, Presentation::Show)
                {
                    return;
                }
                drop(held);
                settle_overlay(&app);
            });
        });
    });
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
mod tests;
