//! Looks for calls once a second while meetings and call prompts are on,
//! and shows the prompt `calls.rs` decides on. On a Mac, Core Audio tells
//! which processes use the microphone from macOS 14, on Apple silicon builds,
//! which have the bridge. On Windows, the microphone consent store does
//! (`call_probe_windows.rs`). Elsewhere there is no detection and no setting.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::{debug, error, info};
use once_cell::sync::Lazy;
use tauri::{AppHandle, Manager};

#[cfg(target_os = "windows")]
use super::call_probe_windows;
use super::calls::{
    self, Action, Browser, CallAnswer, CallApp, CallWatch, MicUser, PromptKind, Snapshot,
};
use super::coordinator::{MeetingAlert, MeetingError};
use super::MeetingsManager;
#[cfg(target_os = "macos")]
use crate::accessibility::{self, Element};
use crate::overlay;
use crate::settings::{get_settings, AppSettings};
use crate::user_alert::{self, Alert};

const POLL: Duration = Duration::from_secs(1);

static RUNNING: AtomicBool = AtomicBool::new(false);
/// Held while the prompt is shown or hidden, so the window follows the
/// watch in the order it changed.
static WATCH: Lazy<Mutex<CallWatch>> = Lazy::new(Mutex::default);

fn watch() -> MutexGuard<'static, CallWatch> {
    WATCH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
extern "C" {
    /// Writes "pid<TAB>bundle id" lines into `buffer` when they fit, and
    /// returns their length, or -1 before macOS 14.
    fn murmur_mic_users(buffer: *mut std::ffi::c_char, capacity: i32) -> i32;
}

/// The processes other than Murmur using the microphone, or `None` where
/// the system does not tell.
fn mic_users() -> Option<Vec<MicUser>> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let mut buffer = vec![0u8; 4096];
        loop {
            let capacity = i32::try_from(buffer.len()).ok()?;
            // SAFETY: the bridge writes at most `capacity` bytes into the
            // buffer, which outlives the call.
            let len = unsafe { murmur_mic_users(buffer.as_mut_ptr().cast(), capacity) };
            let len = usize::try_from(len).ok()?;
            if len < buffer.len() {
                return Some(calls::parse_mic_users(&String::from_utf8_lossy(
                    &buffer[..len],
                )));
            }
            buffer.resize(len + 1, 0);
        }
    }
    #[cfg(target_os = "windows")]
    {
        call_probe_windows::mic_users_now()
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        target_os = "windows"
    )))]
    None
}

/// Whether this computer can tell that a call started.
pub fn supported() -> bool {
    #[cfg(target_os = "windows")]
    {
        call_probe_windows::supported()
    }
    #[cfg(not(target_os = "windows"))]
    {
        mic_users().is_some()
    }
}

fn watching(settings: &AppSettings) -> bool {
    settings.meetings_enabled && settings.meeting_call_prompt
}

fn wanted(app: &AppHandle) -> bool {
    watching(&get_settings(app))
}

/// Starts the watcher when meetings and call prompts are on. It ends by
/// itself, hiding its prompt, once either is turned off.
pub fn sync(app: &AppHandle) {
    if !wanted(app) || !supported() || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("call-watch".into())
        .spawn(move || loop {
            let mut seen = Vec::new();
            let mut browser_calls = BTreeMap::new();
            while wanted(&app) {
                std::thread::sleep(POLL);
                // A panic would leave the watcher marked running, and dead.
                let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    tick(&app, &mut seen, &mut browser_calls)
                }));
                if polled.is_err() {
                    error!("Looking for calls failed");
                }
            }
            {
                let mut watch = watch();
                *watch = CallWatch::default();
                overlay::hide_call_prompt(&app);
            }
            RUNNING.store(false, Ordering::SeqCst);
            // Turned back on before the flag dropped: nobody else started.
            if !wanted(&app) || RUNNING.swap(true, Ordering::SeqCst) {
                return;
            }
        });
    if let Err(error) = spawned {
        RUNNING.store(false, Ordering::SeqCst);
        error!("Failed to start the call watcher: {error}");
    }
}

/// `seen`: the bundle IDs on the microphone at the last poll, logged when
/// they change so an app missing from the table can be found.
/// `browser_calls`: the call each browser on the microphone joined.
fn tick(app: &AppHandle, seen: &mut Vec<String>, browser_calls: &mut BTreeMap<Browser, CallApp>) {
    let Some(users) = mic_users() else {
        return;
    };
    let mut ids: Vec<String> = users.iter().map(|user| user.bundle_id.clone()).collect();
    ids.sort();
    ids.dedup();
    if ids != *seen {
        debug!("Microphone in use by {ids:?}");
        *seen = ids;
    }
    let own_pid = i32::try_from(std::process::id()).unwrap_or(-1);
    let snapshot = Snapshot {
        calls: calls::calls_on_mic(&users, own_pid, browser_calls, browser_call),
        recording: app
            .try_state::<Arc<MeetingsManager>>()
            .is_some_and(|manager| manager.is_recording()),
        now: Instant::now(),
    };
    let mut watch = watch();
    match watch.update(&snapshot) {
        Some(Action::Prompt(kind, call)) => {
            match kind {
                PromptKind::Start => info!("{} started a call: offering to record it", call.name()),
                PromptKind::Ended => info!("{} left the call: offering to stop", call.name()),
            }
            overlay::show_call_prompt(app, kind);
        }
        Some(Action::Hide) => overlay::hide_call_prompt(app),
        None => {}
    }
}

/// The meeting `browser` shows, read through Accessibility from the titles
/// of its windows or the address of its front page. Nothing is logged.
#[cfg(target_os = "macos")]
fn browser_call(browser: Browser) -> Option<CallApp> {
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    workspace.runningApplications().iter().find_map(|running| {
        let bundle_id = running.bundleIdentifier()?.to_string();
        if bundle_id != browser.bundle_id() {
            return None;
        }
        let app = Element::application(running.processIdentifier())?;
        calls::meeting_page(&accessibility::window_titles(&app), || {
            browser
                .reads_url()
                .then(|| accessibility::front_page_url(&app))
                .flatten()
        })
    })
}

/// The meeting `browser` shows on Windows: a window title, or now and then
/// the address bar. The reader remembers when it last read each address.
#[cfg(target_os = "windows")]
fn browser_call(browser: Browser) -> Option<CallApp> {
    static PAGES: Lazy<Mutex<call_probe_windows::PageReader>> = Lazy::new(Mutex::default);
    PAGES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .browser_call(browser, Instant::now())
}

/// Starts the meeting for a call, or tells why it did not.
fn start_meeting(app: &AppHandle) {
    let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
        return;
    };
    if manager.is_recording() {
        return;
    }
    match manager.start() {
        Ok(()) => {}
        Err(error) if lost_a_start_race(&error) => {}
        Err(error) => {
            error!("Recording the call failed: {error}");
            user_alert::show(app, Alert::Meeting(MeetingAlert::StartFailed));
        }
    }
}

/// A start by hand took the coordinator first: the call is being recorded,
/// so there is nothing to tell.
fn lost_a_start_race(error: &str) -> bool {
    error == MeetingError::AlreadyRecording.to_string()
}

/// The prompt's answer. It acts only while the prompt it answers still
/// shows, which it then hides: a meeting stops only on "Stop and
/// transcribe" to the prompt that offered it.
pub fn answer(app: &AppHandle, answer: CallAnswer) {
    {
        let mut watch = watch();
        if watch.answered(answer.kind()).is_none() {
            return;
        }
        overlay::hide_call_prompt(app);
    }
    let app = app.clone();
    match answer {
        CallAnswer::NotNow | CallAnswer::KeepRecording => {}
        CallAnswer::Record => {
            std::thread::spawn(move || start_meeting(&app));
        }
        CallAnswer::StopAndTranscribe => {
            std::thread::spawn(move || {
                let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
                    return;
                };
                if !manager.is_recording() {
                    return;
                }
                if let Err(error) = manager.stop() {
                    error!("Stopping the meeting after its call failed: {error}");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;

    #[test]
    fn a_start_that_lost_the_race_to_a_manual_start_is_not_a_failure() {
        let from_the_coordinator = MeetingError::AlreadyRecording.to_string();
        assert!(lost_a_start_race(&from_the_coordinator));
        assert!(!lost_a_start_race(
            &MeetingError::NoSource("no microphone".into()).to_string()
        ));
        assert!(!lost_a_start_race("Meetings are turned off"));
    }

    /// Windows names the app on the microphone by its executable, with no
    /// process: the same decisions run on every host.
    fn app(id: &str) -> MicUser {
        MicUser {
            pid: 0,
            bundle_id: id.to_string(),
        }
    }

    #[test]
    fn a_browser_is_read_once_while_it_holds_the_microphone_and_shows_its_call() {
        let mut browser_calls = BTreeMap::new();
        let mut reads = Vec::new();
        let mut on_mic = |ids: &[&str], reads: &mut Vec<Browser>| {
            let users: Vec<MicUser> = ids.iter().map(|id| app(id)).collect();
            calls::calls_on_mic(&users, 4242, &mut browser_calls, |browser| {
                reads.push(browser);
                Some(CallApp::GoogleMeet)
            })
        };
        assert_eq!(on_mic(&["chrome.exe"], &mut reads), [CallApp::GoogleMeet]);
        assert_eq!(on_mic(&["chrome.exe"], &mut reads), [CallApp::GoogleMeet]);
        assert_eq!(reads, [Browser::Chrome]);

        assert_eq!(on_mic(&[], &mut reads), []);
        assert_eq!(on_mic(&["chrome.exe"], &mut reads), [CallApp::GoogleMeet]);
        assert_eq!(
            reads,
            [Browser::Chrome, Browser::Chrome],
            "a browser that left the microphone is read again"
        );
    }

    #[test]
    fn apps_the_tables_do_not_know_are_no_call_and_read_nothing() {
        let users = [app("voicerecorder.exe"), app("Contoso.App_abc")];
        let mut reads = 0;
        let found = calls::calls_on_mic(&users, 4242, &mut BTreeMap::new(), |_| {
            reads += 1;
            None
        });
        assert_eq!(found, []);
        assert_eq!(reads, 0);
    }

    #[test]
    fn the_process_murmur_runs_as_is_never_a_call() {
        let own = MicUser {
            pid: 4242,
            bundle_id: "zoom.exe".into(),
        };
        let found = calls::calls_on_mic(&[own], 4242, &mut BTreeMap::new(), |_| None);
        assert_eq!(found, []);
        let found = calls::calls_on_mic(&[app("zoom.exe")], 4242, &mut BTreeMap::new(), |_| None);
        assert_eq!(found, [CallApp::Zoom]);
    }

    #[test]
    fn calls_are_watched_only_while_call_prompts_and_meetings_are_on() {
        let mut settings = get_default_settings();
        settings.meetings_enabled = true;
        settings.meeting_call_prompt = true;
        assert!(watching(&settings));

        settings.meeting_call_prompt = false;
        assert!(!watching(&settings));

        settings.meeting_call_prompt = true;
        settings.meetings_enabled = false;
        assert!(!watching(&settings));
    }
}
