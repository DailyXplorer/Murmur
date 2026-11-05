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
            let mut poll = Poll::default();
            let mut system = System::default();
            while wanted(&app) {
                std::thread::sleep(POLL);
                // A panic would leave the watcher marked running, and dead.
                let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    tick(&app, &mut poll, &mut system)
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

/// What the watcher asks the system each second. A test answers in its
/// place.
trait Probe {
    /// The processes other than Murmur on the microphone, or `None` where
    /// the system does not tell.
    fn mic_users(&mut self) -> Option<Vec<MicUser>>;
    /// The meeting `browser` shows, if it shows one. Nothing is logged.
    fn browser_call(&mut self, browser: Browser) -> Option<CallApp>;
}

/// The computer the watcher runs on.
#[derive(Default)]
struct System {
    #[cfg(target_os = "windows")]
    pages: call_probe_windows::PageReader,
}

impl Probe for System {
    fn mic_users(&mut self) -> Option<Vec<MicUser>> {
        mic_users()
    }

    fn browser_call(&mut self, browser: Browser) -> Option<CallApp> {
        #[cfg(target_os = "macos")]
        {
            browser_call(browser)
        }
        #[cfg(target_os = "windows")]
        {
            self.pages.browser_call(browser, Instant::now())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = browser;
            None
        }
    }
}

/// What carries from one second's poll to the next.
#[derive(Default)]
struct Poll {
    /// The app ids on the microphone at the last poll, logged when they
    /// change so an app missing from the tables can be found.
    seen: Vec<String>,
    /// The call each browser on the microphone joined.
    browser_calls: BTreeMap<Browser, CallApp>,
}

impl Poll {
    /// This second's reading, or `None` where the system does not tell.
    fn snapshot(
        &mut self,
        probe: &mut impl Probe,
        own_pid: i32,
        recording: bool,
        now: Instant,
    ) -> Option<Snapshot> {
        let users = probe.mic_users()?;
        let mut ids: Vec<String> = users.iter().map(|user| user.bundle_id.clone()).collect();
        ids.sort();
        ids.dedup();
        if ids != self.seen {
            debug!("Microphone in use by {ids:?}");
            self.seen = ids;
        }
        let calls = calls::calls_on_mic(&users, own_pid, &mut self.browser_calls, |browser| {
            probe.browser_call(browser)
        });
        Some(Snapshot {
            calls,
            recording,
            now,
        })
    }
}

fn tick(app: &AppHandle, poll: &mut Poll, system: &mut System) {
    let own_pid = i32::try_from(std::process::id()).unwrap_or(-1);
    let recording = app
        .try_state::<Arc<MeetingsManager>>()
        .is_some_and(|manager| manager.is_recording());
    let Some(snapshot) = poll.snapshot(system, own_pid, recording, Instant::now()) else {
        return;
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

    const OWN_PID: i32 = 4242;

    /// A computer whose microphone and browser pages the test sets.
    struct Fake {
        tells: bool,
        users: Vec<MicUser>,
        page: Option<CallApp>,
        page_reads: Vec<Browser>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                tells: true,
                users: Vec::new(),
                page: None,
                page_reads: Vec::new(),
            }
        }
    }

    impl Probe for Fake {
        fn mic_users(&mut self) -> Option<Vec<MicUser>> {
            self.tells.then(|| self.users.clone())
        }

        fn browser_call(&mut self, browser: Browser) -> Option<CallApp> {
            self.page_reads.push(browser);
            self.page
        }
    }

    fn apps(ids: &[&str]) -> Vec<MicUser> {
        ids.iter()
            .map(|id| MicUser {
                pid: 0,
                bundle_id: (*id).to_string(),
            })
            .collect()
    }

    /// The watcher's loop without the app: one poll a second.
    struct Run {
        watch: CallWatch,
        poll: Poll,
        probe: Fake,
        start: Instant,
        second: u64,
    }

    impl Run {
        fn new() -> Self {
            Self {
                watch: CallWatch::default(),
                poll: Poll::default(),
                probe: Fake::new(),
                start: Instant::now(),
                second: 0,
            }
        }

        /// `seconds` polls with `ids` on the microphone, and each action
        /// they returned with the second it came at.
        fn run(&mut self, seconds: u64, ids: &[&str], recording: bool) -> Vec<(u64, Action)> {
            self.probe.users = apps(ids);
            (0..seconds)
                .filter_map(|_| {
                    let at = self.second;
                    self.second += 1;
                    let now = self.start + Duration::from_secs(at);
                    let snapshot = self
                        .poll
                        .snapshot(&mut self.probe, OWN_PID, recording, now)?;
                    self.watch.update(&snapshot).map(|action| (at, action))
                })
                .collect()
        }
    }

    const START_ZOOM: Action = Action::Prompt(PromptKind::Start, CallApp::Zoom);
    const ENDED_ZOOM: Action = Action::Prompt(PromptKind::Ended, CallApp::Zoom);

    #[test]
    fn a_call_held_three_seconds_asks_and_the_question_closes_fifteen_seconds_after_it_left() {
        let mut run = Run::new();
        assert_eq!(run.run(3, &["zoom.exe"], false), []);
        assert_eq!(run.run(1, &["zoom.exe"], false), [(3, START_ZOOM)]);
        assert_eq!(run.run(6, &["zoom.exe"], false), []);

        assert_eq!(run.run(15, &[], false), [], "a mute is not the end");
        assert_eq!(run.run(1, &[], false), [(25, Action::Hide)]);
        assert_eq!(run.run(30, &[], false), []);
    }

    #[test]
    fn a_call_that_ends_while_the_meeting_records_asks_to_stop_and_keep_recording_asks_once() {
        let mut run = Run::new();
        run.run(4, &["zoom.exe"], false);
        assert_eq!(
            run.watch.answered(PromptKind::Start),
            Some(CallApp::Zoom),
            "Record"
        );
        assert_eq!(run.run(20, &["zoom.exe"], true), []);

        assert_eq!(run.run(3, &[], true), []);
        assert_eq!(run.run(1, &[], true), [(27, ENDED_ZOOM)]);
        assert_eq!(run.run(600, &[], true), [], "it never goes on its own");

        assert_eq!(
            run.watch.answered(PromptKind::Ended),
            Some(CallApp::Zoom),
            "Keep recording"
        );
        assert_eq!(run.run(600, &[], true), [], "not asked again");

        assert_eq!(run.run(5, &["zoom.exe"], true), []);
        let asked_again = run.run(4, &[], true);
        assert_eq!(asked_again.len(), 1, "only a new call asks again");
        assert_eq!(asked_again[0].1, ENDED_ZOOM);
    }

    #[test]
    fn a_browser_is_read_once_while_it_holds_the_microphone_and_shows_its_call() {
        let mut run = Run::new();
        run.probe.page = Some(CallApp::GoogleMeet);
        assert_eq!(
            run.run(5, &["chrome.exe"], false),
            [(3, Action::Prompt(PromptKind::Start, CallApp::GoogleMeet))]
        );
        assert_eq!(run.probe.page_reads, [Browser::Chrome]);

        run.run(1, &[], false);
        run.run(1, &["chrome.exe"], false);
        assert_eq!(
            run.probe.page_reads,
            [Browser::Chrome, Browser::Chrome],
            "a browser that left the microphone is read again"
        );
    }

    #[test]
    fn a_browser_on_the_microphone_without_a_meeting_page_is_no_call() {
        let mut run = Run::new();
        assert_eq!(run.run(30, &["msedge.exe"], false), []);
        assert_eq!(run.probe.page_reads.len(), 30, "read every second");
        assert!(run
            .probe
            .page_reads
            .iter()
            .all(|read| *read == Browser::Edge));
    }

    #[test]
    fn apps_the_tables_do_not_know_are_no_call_and_read_nothing() {
        let mut run = Run::new();
        let ids = ["voicerecorder.exe", "Contoso.App_abc"];
        assert_eq!(run.run(30, &ids, false), []);
        assert!(run.probe.page_reads.is_empty());
    }

    #[test]
    fn a_computer_that_cannot_tell_asks_nothing() {
        let mut run = Run::new();
        run.probe.tells = false;
        assert_eq!(run.run(30, &["zoom.exe"], false), []);
        assert!(run.probe.page_reads.is_empty());
    }

    #[test]
    fn the_process_murmur_runs_as_is_never_a_call() {
        let mut poll = Poll::default();
        let mut probe = Fake::new();
        probe.users = vec![MicUser {
            pid: OWN_PID,
            bundle_id: "zoom.exe".into(),
        }];
        let snapshot = poll
            .snapshot(&mut probe, OWN_PID, false, Instant::now())
            .unwrap();
        assert_eq!(snapshot.calls, []);

        probe.users = apps(&["zoom.exe"]);
        let snapshot = poll
            .snapshot(&mut probe, OWN_PID, false, Instant::now())
            .unwrap();
        assert_eq!(snapshot.calls, [CallApp::Zoom]);
    }
}
