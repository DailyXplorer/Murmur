//! Looks for calls once a second while meetings and call prompts are on,
//! and shows the prompt `calls.rs` decides on. Core Audio tells which
//! processes use the microphone from macOS 14, on Apple silicon builds,
//! which have the bridge; elsewhere there is no detection and no setting.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use log::{debug, error, info};
use once_cell::sync::Lazy;
use tauri::{AppHandle, Emitter, Manager};

use super::calls::{
    self, Action, Browser, CallAnswer, CallApp, CallPolicy, CallWatch, MicUser, PromptKind,
    Snapshot,
};
use super::coordinator::MeetingAlert;
use super::MeetingsManager;
use crate::accessibility::{self, Element};
use crate::overlay;
use crate::settings::{get_settings, update_settings, AppSettings};
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

/// The meeting a rule started, which "Don't record" discards: never one the
/// user started since.
static STARTED_BY_RULE: Mutex<Option<i64>> = Mutex::new(None);

fn started_by_rule() -> MutexGuard<'static, Option<i64>> {
    STARTED_BY_RULE
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

/// The switch for call prompts also stops the apps' rules: an app set to
/// "Always record" records nothing while it is off.
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
        policies: get_settings(app).meeting_call_policies,
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
                PromptKind::Started => {}
            }
            overlay::show_call_prompt(app, kind, call.name());
        }
        Some(Action::Start(call)) => {
            info!(
                "{} started a call: recording it as its rule says",
                call.name()
            );
            match start_meeting(app) {
                Some(id) => {
                    *started_by_rule() = Some(id);
                    overlay::show_call_prompt(app, PromptKind::Started, call.name());
                }
                None => {
                    watch.answered(PromptKind::Started);
                }
            }
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

/// Starts the meeting for a call, or tells why it did not. `None` when one
/// already records or the start failed.
fn start_meeting(app: &AppHandle) -> Option<i64> {
    let manager = app.try_state::<Arc<MeetingsManager>>()?;
    if manager.is_recording() {
        return None;
    }
    match manager.start_returning_id() {
        Ok(id) => Some(id),
        Err(error) => {
            error!("Recording the call failed: {error}");
            user_alert::show(app, Alert::Meeting(MeetingAlert::StartFailed));
            None
        }
    }
}

/// Keeps the rule the prompt's button chose, and tells an open settings
/// window.
fn set_policy(app: &AppHandle, call: CallApp, policy: CallPolicy) {
    update_settings(app, |settings| {
        settings.meeting_call_policies.insert(call, policy);
    });
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"setting": "meeting_call_policies"}),
    );
}

/// The prompt's answer. It acts only while the prompt it answers still
/// shows, which it then hides: a meeting stops only on "Stop and
/// transcribe" to the prompt that offered it, and one is discarded only on
/// "Don't record" to the prompt of the meeting a rule started.
pub fn answer(app: &AppHandle, answer: CallAnswer) {
    let call = {
        let mut watch = watch();
        let Some(call) = watch.answered(answer.kind()) else {
            return;
        };
        overlay::hide_call_prompt(app);
        call
    };
    let app = app.clone();
    match answer {
        CallAnswer::NotNow | CallAnswer::KeepRecording => {}
        CallAnswer::NeverAsk => set_policy(&app, call, CallPolicy::Never),
        CallAnswer::Record | CallAnswer::AlwaysRecord => {
            if answer == CallAnswer::AlwaysRecord {
                set_policy(&app, call, CallPolicy::Always);
            }
            std::thread::spawn(move || {
                start_meeting(&app);
            });
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
        CallAnswer::DontRecord => {
            std::thread::spawn(move || {
                let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
                    return;
                };
                let Some(id) = started_by_rule().take() else {
                    return;
                };
                match manager.discard(id) {
                    Ok(()) => info!("Meeting {id} discarded: not recorded after all"),
                    Err(error) => error!("Discarding meeting {id} failed: {error}"),
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
    fn a_rule_records_nothing_while_call_prompts_or_meetings_are_off() {
        let mut settings = get_default_settings();
        settings
            .meeting_call_policies
            .insert(CallApp::Zoom, CallPolicy::Always);
        assert!(watching(&settings));

        settings.meeting_call_prompt = false;
        assert!(!watching(&settings));

        settings.meeting_call_prompt = true;
        settings.meetings_enabled = false;
        assert!(!watching(&settings));
    }
}
