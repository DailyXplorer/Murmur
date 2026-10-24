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
use tauri::{AppHandle, Manager};

use super::calls::{self, Action, Browser, CallApp, CallWatch, MicUser, Snapshot};
use super::coordinator::MeetingAlert;
use super::MeetingsManager;
#[cfg(target_os = "macos")]
use crate::accessibility::{self, Element};
use crate::overlay;
use crate::settings::get_settings;
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
        Some(Action::Prompt(call)) => {
            info!("{} started a call: offering to record it", call.name());
            overlay::show_call_prompt(app, call.name());
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

/// Windows has no call detection: `mic_users` never answers there.
#[cfg(target_os = "windows")]
fn browser_call(_browser: Browser) -> Option<CallApp> {
    None
}

/// The prompt's answer, "Record" when `accept`. "Record" acts only while
/// the prompt still shows; every answer hides it.
pub fn answer(app: &AppHandle, accept: bool) {
    let answered = {
        let mut watch = watch();
        overlay::hide_call_prompt(app);
        watch.answered()
    };
    if answered.is_none() || !accept {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
            return;
        };
        if manager.is_recording() {
            return;
        }
        if let Err(error) = manager.start() {
            error!("Recording the call failed: {error}");
            user_alert::show(&app, Alert::Meeting(MeetingAlert::StartFailed));
        }
    });
}
