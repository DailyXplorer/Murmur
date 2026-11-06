//! What the settings window costs while nobody looks at it. Once the
//! shortcuts run in Rust and the page left onboarding, a closed window is
//! destroyed with its web process; the next open builds a fresh one on the
//! section the user left, and pulls what happened while it was gone.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use crate::cli::CliArgs;
use crate::commands::ShortcutsInitialized;
use crate::meetings::announce::OpenTarget;

pub const MAIN_WINDOW: &str = "main";

/// What the settings page shows, as it reports it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SettingsView {
    Onboarding,
    Section(String),
}

/// Something that happened while the settings window was gone, for the
/// next one to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum MainAction {
    CheckUpdates,
    SwitchedToLocal,
    OpenMeetings(OpenTarget),
}

#[derive(Debug, PartialEq, Eq)]
pub enum CloseAction {
    Destroy,
    Hide,
}

/// Without the tray there is no way back but the Dock, and before the
/// shortcuts run only the page can start them; a page on onboarding holds
/// the user's progress, and a held page runs work only it can finish, such
/// as an update install that ends in its relaunch.
pub fn close_action(
    tray_visible: bool,
    shortcuts_ready: bool,
    onboarding: bool,
    held: bool,
) -> CloseAction {
    if tray_visible && shortcuts_ready && !onboarding && !held {
        CloseAction::Destroy
    } else {
        CloseAction::Hide
    }
}

#[derive(Debug)]
struct Inner {
    view: SettingsView,
    opened_meeting: Option<i64>,
    last_section: Option<String>,
    pending: Vec<MainAction>,
    holds: u32,
}

/// Lives as long as the app, not the window. Kept in memory only.
#[derive(Debug)]
pub struct SettingsWindow(Mutex<Inner>);

impl Default for SettingsWindow {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            view: SettingsView::Onboarding,
            opened_meeting: None,
            last_section: None,
            pending: Vec::new(),
            holds: 0,
        }))
    }
}

impl SettingsWindow {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn report(&self, view: SettingsView) {
        let mut inner = self.lock();
        if view != SettingsView::Section("meetings".to_string()) {
            inner.opened_meeting = None;
        }
        if let SettingsView::Section(section) = &view {
            inner.last_section = Some(section.clone());
        }
        inner.view = view;
    }

    fn onboarding(&self) -> bool {
        self.lock().view == SettingsView::Onboarding
    }

    fn report_meeting(&self, id: Option<i64>) {
        self.lock().opened_meeting = id;
    }

    #[cfg(any(target_os = "macos", test))]
    fn viewed_meeting(&self, visible: bool, on_active_space: bool, minimized: bool) -> Option<i64> {
        let inner = self.lock();
        if inner.view != SettingsView::Section("meetings".to_string()) {
            return None;
        }
        inner.opened_meeting.filter(|id| {
            suppress_meeting_notice(
                visible,
                on_active_space,
                minimized,
                inner.opened_meeting,
                *id,
            )
        })
    }

    fn held(&self) -> bool {
        self.lock().holds > 0
    }

    fn hold(&self) {
        self.lock().holds += 1;
    }

    fn release(&self) {
        let mut inner = self.lock();
        inner.holds = inner.holds.saturating_sub(1);
    }

    /// The page's view and its holds go with it.
    fn forget_page(&self) {
        let mut inner = self.lock();
        inner.view = SettingsView::Onboarding;
        inner.opened_meeting = None;
        inner.holds = 0;
    }

    fn last_section(&self) -> Option<String> {
        self.lock().last_section.clone()
    }

    fn queue(&self, action: MainAction) {
        let mut inner = self.lock();
        if matches!(action, MainAction::OpenMeetings(_)) {
            inner
                .pending
                .retain(|pending| !matches!(pending, MainAction::OpenMeetings(_)));
        }
        if !inner.pending.contains(&action) {
            inner.pending.push(action);
        }
    }

    fn take(&self, action: MainAction) -> bool {
        let mut inner = self.lock();
        let before = inner.pending.len();
        inner.pending.retain(|pending| *pending != action);
        inner.pending.len() != before
    }

    fn take_meetings(&self) -> Option<OpenTarget> {
        let mut inner = self.lock();
        let index = inner
            .pending
            .iter()
            .position(|action| matches!(action, MainAction::OpenMeetings(_)))?;
        match inner.pending.remove(index) {
            MainAction::OpenMeetings(target) => Some(target),
            _ => None,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn suppress_meeting_notice(
    visible: bool,
    on_active_space: bool,
    minimized: bool,
    reported_meeting: Option<i64>,
    finished_id: i64,
) -> bool {
    visible && on_active_space && !minimized && reported_meeting == Some(finished_id)
}

/// Main thread only, like `prepare_settings_window_for_active_space`.
#[cfg(target_os = "macos")]
pub fn viewed_meeting(app: &AppHandle) -> Option<i64> {
    let window = app.get_webview_window(MAIN_WINDOW)?;
    let native_window = window.ns_window().ok()?.cast::<objc2_app_kit::NSWindow>();
    let native_window = unsafe { &*native_window };
    app.try_state::<SettingsWindow>().and_then(|state| {
        state.viewed_meeting(
            native_window.isVisible(),
            native_window.isOnActiveSpace(),
            native_window.isMiniaturized(),
        )
    })
}

/// Meetings are off on Windows, so none is viewed.
#[cfg(target_os = "windows")]
pub fn viewed_meeting(_app: &AppHandle) -> Option<i64> {
    None
}

pub fn tray_visible(app: &AppHandle) -> bool {
    crate::settings::get_settings(app).show_tray_icon && !app.state::<CliArgs>().no_tray
}

/// What closing the settings window does now.
pub fn current_close_action(app: &AppHandle) -> CloseAction {
    let state = app.try_state::<SettingsWindow>();
    close_action(
        tray_visible(app),
        app.try_state::<ShortcutsInitialized>().is_some(),
        state.as_ref().is_none_or(|state| state.onboarding()),
        state.as_ref().is_some_and(|state| state.held()),
    )
}

/// A window gone takes its page's view and holds with it: the next page
/// reports its own view once it knows whether onboarding is needed.
pub fn on_destroyed(app: &AppHandle) {
    if let Some(state) = app.try_state::<SettingsWindow>() {
        state.forget_page();
    }
}

/// Destroys a settings window nobody sees, such as the one a hidden start
/// builds to run the onboarding checks and start the shortcuts, or one
/// closed while held. Callers are that page's own commands, so the destroy
/// is scheduled on the main thread rather than run inside them.
pub fn release_if_hidden(app: &AppHandle) {
    let handle = app.clone();
    let scheduled = app.run_on_main_thread(move || {
        if current_close_action(&handle) != CloseAction::Destroy {
            return;
        }
        let Some(window) = handle.get_webview_window(MAIN_WINDOW) else {
            return;
        };
        if window.is_visible().unwrap_or(true) {
            return;
        }
        log::info!("Releasing the hidden settings window");
        if let Err(error) = window.destroy() {
            log::error!("Failed to release the hidden settings window: {error}");
        }
    });
    if let Err(error) = scheduled {
        log::error!("Failed to schedule the settings window release: {error}");
    }
}

/// Keeps `action` until a settings window takes it. Its event still goes
/// out: the window that hears it takes the action then, and one that was
/// closed or still loading takes it once it listens.
pub fn queue(app: &AppHandle, action: MainAction) {
    if let Some(state) = app.try_state::<SettingsWindow>() {
        state.queue(action);
    }
}

#[tauri::command]
#[specta::specta]
pub fn report_settings_view(app: AppHandle, view: SettingsView) {
    if let Some(state) = app.try_state::<SettingsWindow>() {
        state.report(view);
    }
    crate::overlay::settings_meeting_changed(&app);
    release_if_hidden(&app);
}

#[tauri::command]
#[specta::specta]
pub fn report_opened_meeting(app: AppHandle, id: Option<i64>) {
    if let Some(state) = app.try_state::<SettingsWindow>() {
        state.report_meeting(id);
    }
    crate::overlay::settings_meeting_changed(&app);
}

#[tauri::command]
#[specta::specta]
pub fn take_meetings_action(app: AppHandle) -> Option<OpenTarget> {
    app.try_state::<SettingsWindow>()
        .and_then(|state| state.take_meetings())
}

/// Until the matching release, closing the settings window only hides it:
/// the page runs work that would die with it.
#[tauri::command]
#[specta::specta]
pub fn hold_settings_window(app: AppHandle) {
    if let Some(state) = app.try_state::<SettingsWindow>() {
        state.hold();
    }
}

/// Ends one hold. A window closed while held is destroyed once none is left.
#[tauri::command]
#[specta::specta]
pub fn release_settings_window(app: AppHandle) {
    if let Some(state) = app.try_state::<SettingsWindow>() {
        state.release();
    }
    release_if_hidden(&app);
}

#[tauri::command]
#[specta::specta]
pub fn get_last_settings_section(app: AppHandle) -> Option<String> {
    app.try_state::<SettingsWindow>()
        .and_then(|state| state.last_section())
}

/// True once per `action` kept for this window.
#[tauri::command]
#[specta::specta]
pub fn take_main_action(app: AppHandle, action: MainAction) -> bool {
    app.try_state::<SettingsWindow>()
        .is_some_and(|state| state.take(action))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_destroys_when_tray_and_ready() {
        assert_eq!(close_action(true, true, false, false), CloseAction::Destroy);
    }

    #[test]
    fn close_hides_without_tray() {
        assert_eq!(close_action(false, true, false, false), CloseAction::Hide);
    }

    #[test]
    fn close_hides_before_shortcuts_ready() {
        assert_eq!(close_action(true, false, false, false), CloseAction::Hide);
    }

    #[test]
    fn close_hides_during_onboarding() {
        assert_eq!(close_action(true, true, true, false), CloseAction::Hide);
    }

    #[test]
    fn close_hides_while_held() {
        assert_eq!(close_action(true, true, false, true), CloseAction::Hide);
    }

    #[test]
    fn holds_end_with_their_releases_or_the_page() {
        let state = SettingsWindow::default();
        state.hold();
        state.hold();
        state.release();
        assert!(state.held());
        state.release();
        assert!(!state.held());
        state.release();
        assert!(!state.held());

        state.hold();
        state.forget_page();
        assert!(!state.held());
    }

    #[test]
    fn pending_action_is_taken_once() {
        let state = SettingsWindow::default();
        state.queue(MainAction::CheckUpdates);
        state.queue(MainAction::CheckUpdates);
        state.queue(MainAction::SwitchedToLocal);

        assert!(state.take(MainAction::CheckUpdates));
        assert!(!state.take(MainAction::CheckUpdates));
        assert!(state.take(MainAction::SwitchedToLocal));
        assert!(!state.take(MainAction::SwitchedToLocal));
    }

    #[test]
    fn last_section_outlives_the_window() {
        let state = SettingsWindow::default();
        assert!(state.onboarding());
        state.report(SettingsView::Section("history".into()));
        assert!(!state.onboarding());

        state.forget_page();

        assert!(state.onboarding());
        assert_eq!(state.last_section().as_deref(), Some("history"));
    }

    #[test]
    fn only_a_visible_meeting_page_suppresses_its_announcement() {
        let state = SettingsWindow::default();
        state.report(SettingsView::Section("meetings".into()));
        state.report_meeting(Some(7));
        assert_eq!(state.viewed_meeting(true, true, false), Some(7));
        assert_eq!(state.viewed_meeting(false, true, false), None);
        assert_eq!(state.viewed_meeting(true, false, false), None);
        assert_eq!(state.viewed_meeting(true, true, true), None);
        state.report_meeting(None);
        assert_eq!(state.viewed_meeting(true, true, false), None);
        state.report_meeting(Some(7));
        state.report(SettingsView::Section("general".into()));
        assert_eq!(state.viewed_meeting(true, true, false), None);
        state.report(SettingsView::Section("meetings".into()));
        assert_eq!(state.viewed_meeting(true, true, false), None);
        state.report_meeting(Some(7));
        state.forget_page();
        assert_eq!(state.viewed_meeting(true, true, false), None);
    }

    #[test]
    fn suppression_requires_the_finished_meeting_on_the_active_space() {
        assert!(suppress_meeting_notice(true, true, false, Some(7), 7));
        for (visible, active, minimized, reported, finished) in [
            (false, true, false, Some(7), 7),
            (true, false, false, Some(7), 7),
            (true, true, true, Some(7), 7),
            (true, true, false, None, 7),
            (true, true, false, Some(8), 7),
            (true, true, false, Some(7), 8),
        ] {
            assert!(!suppress_meeting_notice(
                visible, active, minimized, reported, finished
            ));
        }
    }

    #[test]
    fn the_latest_meeting_open_survives_a_destroyed_window_and_is_taken_once() {
        let state = SettingsWindow::default();
        state.queue(MainAction::CheckUpdates);
        state.queue(MainAction::OpenMeetings(OpenTarget::Meeting(7)));
        state.forget_page();
        assert_eq!(state.take_meetings(), Some(OpenTarget::Meeting(7)));
        assert_eq!(state.take_meetings(), None);
        state.queue(MainAction::OpenMeetings(OpenTarget::Meeting(8)));
        state.queue(MainAction::OpenMeetings(OpenTarget::Meetings));
        assert_eq!(state.take_meetings(), Some(OpenTarget::Meetings));
        assert_eq!(state.take_meetings(), None);
        assert!(state.take(MainAction::CheckUpdates));
    }
}
