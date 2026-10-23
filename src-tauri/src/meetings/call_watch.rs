//! Looks for calls once a second while meetings and call prompts are on,
//! and shows the prompts `calls.rs` decides on. Core Audio tells which
//! processes use the microphone from macOS 14, on Apple silicon builds,
//! which have the bridge; elsewhere there is no detection and no setting.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::{debug, error, info};
use once_cell::sync::Lazy;
use tauri::{AppHandle, Manager};

use super::calls::{self, Action, Browser, CallApp, CallWatch, MicUser, Question, Snapshot};
use super::coordinator::{MeetingAlert, MeetingRuntime};
use super::MeetingsManager;
use crate::accessibility::{self, Element};
use crate::overlay::{self, CallPrompt};
use crate::settings::get_settings;
use crate::user_alert::{self, Alert};

const POLL: Duration = Duration::from_secs(1);

static RUNNING: AtomicBool = AtomicBool::new(false);
static WATCH: Lazy<Mutex<CallWatch>> = Lazy::new(Mutex::default);

fn watch() -> MutexGuard<'static, CallWatch> {
    WATCH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(target_arch = "aarch64")]
extern "C" {
    /// Writes "pid<TAB>bundle id" lines into `buffer` when they fit, and
    /// returns their length, or -1 before macOS 14.
    fn murmur_mic_users(buffer: *mut std::ffi::c_char, capacity: i32) -> i32;
}

/// The processes other than Murmur using the microphone, or `None` where
/// macOS does not tell.
fn mic_users() -> Option<Vec<MicUser>> {
    #[cfg(target_arch = "aarch64")]
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
    #[cfg(not(target_arch = "aarch64"))]
    None
}

/// Whether this Mac can tell that a call started.
pub fn supported() -> bool {
    mic_users().is_some()
}

fn wanted(app: &AppHandle) -> bool {
    let settings = get_settings(app);
    settings.meetings_enabled && settings.meeting_call_prompt
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
            *watch() = CallWatch::default();
            overlay::hide_call_prompt(&app);
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
    let meeting =
        app.try_state::<Arc<MeetingsManager>>()
            .and_then(|manager| match manager.runtime() {
                MeetingRuntime::Recording {
                    meeting_started_at_ms,
                    ..
                } => Some(meeting_started_at_ms),
                MeetingRuntime::Idle => None,
            });
    let snapshot = Snapshot {
        calls: calls::calls_on_mic(&users, own_pid, browser_calls, browser_call),
        dictating: crate::transcription_coordinator::dictating(),
        meeting,
        now: Instant::now(),
    };
    let action = watch().update(&snapshot);
    match action {
        Some(Action::Prompt(call)) => {
            info!("{} started a call: offering to record it", call.name());
            overlay::show_call_prompt(app, CallPrompt::Record, call.name());
        }
        Some(Action::Ended(call)) => {
            info!("The {} call ended: offering to stop recording", call.name());
            overlay::show_call_prompt(app, CallPrompt::Stop, call.name());
        }
        Some(Action::Hide) => overlay::hide_call_prompt(app),
        None => {}
    }
}

/// The meeting `browser` shows, read through Accessibility from the titles
/// of its windows or the address of its front page. Nothing is logged.
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

/// The overlay's answer to the prompt asking `prompt`. "Record" and "Stop"
/// act only while that prompt still shows; every answer hides it.
pub fn answer(app: &AppHandle, prompt: CallPrompt, accept: bool) {
    let question = match prompt {
        CallPrompt::Record => Question::Record,
        CallPrompt::Stop => Question::Stop,
    };
    let answered = watch().answered(question);
    // Before the meeting starts, so its pill can show.
    overlay::hide_call_prompt(app);
    if answered.is_none() || !accept {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
            return;
        };
        match question {
            Question::Record if !manager.is_recording() => {
                if let Err(error) = manager.start() {
                    error!("Recording the call failed: {error}");
                    user_alert::show(&app, Alert::Meeting(MeetingAlert::StartFailed));
                }
            }
            Question::Stop if manager.is_recording() => {
                if let Err(error) = manager.stop() {
                    error!("Stopping the meeting after the call failed: {error}");
                }
            }
            _ => {}
        }
    });
}
