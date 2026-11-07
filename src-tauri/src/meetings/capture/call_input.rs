//! Which microphone the call app records from, for the "Me" track.
//!
//! A call app often uses another input than the one Murmur would open: a
//! headset or AirPods the call picked, while the system default is the
//! built-in microphone. Recording the default then leaves the user's side
//! quiet or wrong. The system tells which input devices the processes on the
//! microphone read from (Core Audio's per-process device list on macOS 14,
//! WASAPI audio sessions on Windows), and `Router` decides when the track
//! moves to the call's device. Everything here is a plain function or a
//! state machine, tested on every host; only the reads of the system sit
//! behind `cfg`.
//!
//! The default input is never changed: Murmur opens the call's device by
//! name next to the call app, so a Bluetooth headset stays what it was.
//!
//! The choice in Settings is respected: a microphone picked there keeps
//! recording unless it delivers digital silence (an unplugged cable, a
//! hardware mute, the wrong one) while a call app records from another device.
//! "Default" follows the call app. Device names are never logged: they can
//! hold their owner's name.

use std::time::{Duration, Instant};

use crate::meetings::calls::{mic_app, MicApp};

/// How long the call's device must stay different from the open one before the
/// track moves: apps open the microphone in steps, and a headset connecting
/// shows up first as the default.
pub const SETTLE: Duration = Duration::from_secs(2);
/// How long a microphone chosen in Settings must give nothing but silence
/// before the call's device replaces it.
pub const SILENT_AFTER: Duration = Duration::from_secs(10);
/// The loudest sample that still counts as silence (about -70 dBFS): a muted
/// or disconnected input sends zeros, a live one has at least a noise floor.
pub const SILENT_PEAK: f32 = 3e-4;
/// How often the system is asked which devices the call apps use.
pub const POLL: Duration = Duration::from_secs(1);

/// One input device a process on the microphone reads from. `app_id` is a
/// macOS bundle identifier or a Windows executable name, as `calls::mic_app`
/// knows them; `device` is the device's name, which is how cpal names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInput {
    pub app_id: String,
    pub device: String,
}

/// Reads the list `murmur_mic_user_inputs` writes: one
/// "pid<TAB>bundle id<TAB>device name" line per process and device.
#[cfg(any(all(target_os = "macos", target_arch = "aarch64"), test))]
pub fn parse_inputs(list: &str) -> Vec<ProcessInput> {
    list.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let (_pid, app_id, device) = (fields.next()?, fields.next()?, fields.next()?);
            (!app_id.is_empty() && !device.is_empty()).then(|| ProcessInput {
                app_id: app_id.to_string(),
                device: device.to_string(),
            })
        })
        .collect()
}

/// The devices the call apps record from, the likeliest first: the call apps
/// before the browsers (a browser holds the microphone for any page), each
/// group by name, each device once.
pub fn call_devices(users: &[ProcessInput]) -> Vec<String> {
    let mut ranked: Vec<(u8, &str)> = users
        .iter()
        .filter_map(|user| {
            let rank = match mic_app(&user.app_id)? {
                MicApp::Call(_) => 0,
                MicApp::Browser(_) => 1,
            };
            Some((rank, user.device.as_str()))
        })
        .collect();
    ranked.sort();
    let mut devices: Vec<String> = Vec::new();
    for (_, device) in ranked {
        if !devices.iter().any(|known| known == device) {
            devices.push(device.to_string());
        }
    }
    devices
}

/// Where the track records from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Route {
    /// What Settings say: the chosen microphone, else the system default.
    #[default]
    Settings,
    /// The input device a call app uses, by name.
    Call(String),
}

/// What one look at the system found.
pub struct Look<'a> {
    /// `call_devices` now; empty when no call app records or the system does
    /// not tell.
    pub calls: &'a [String],
    /// The name of the device the track records from, when known.
    pub open: Option<&'a str>,
    /// Whether the open device is the microphone chosen in Settings.
    pub explicit: bool,
    /// How long the open device has sent nothing but silence.
    pub quiet: Duration,
}

/// Decides when the track moves to the call's device.
///
/// A call app that stops recording (muted, or between two calls) leaves the
/// track where it is: going back to the default would only flap. A device
/// that disappears is handled by the reopen of `mic.rs`, which falls back to
/// the settings.
#[derive(Debug, Default)]
pub struct Router {
    route: Route,
    /// A device the call uses that differs from the open one, and since when.
    pending: Option<(String, Instant)>,
}

impl Router {
    pub fn route(&self) -> &Route {
        &self.route
    }

    /// The route before the first open. A microphone chosen in Settings keeps
    /// it; otherwise the call's device is taken at once.
    pub fn start(&mut self, calls: &[String], explicit: bool) -> &Route {
        self.pending = None;
        self.route = match calls.first() {
            Some(device) if !explicit => Route::Call(device.clone()),
            _ => Route::Settings,
        };
        &self.route
    }

    /// The device the track should move to, if any, given `look`.
    fn wanted<'a>(&self, look: &Look<'a>) -> Option<&'a String> {
        let first = look.calls.first()?;
        match &self.route {
            Route::Call(current) => (!look.calls.contains(current)).then_some(first),
            Route::Settings => {
                let on_a_call_device = look
                    .open
                    .is_some_and(|open| look.calls.iter().any(|device| device == open));
                let respected = look.explicit && look.quiet < SILENT_AFTER;
                (!on_a_call_device && !respected).then_some(first)
            }
        }
    }

    /// Looks once; returns the new route when the track must reopen on it.
    pub fn observe(&mut self, now: Instant, look: &Look) -> Option<Route> {
        let Some(wanted) = self.wanted(look) else {
            self.pending = None;
            return None;
        };
        match &self.pending {
            Some((device, since)) if device == wanted => {
                if now.saturating_duration_since(*since) < SETTLE {
                    return None;
                }
            }
            _ => {
                self.pending = Some((wanted.clone(), now));
                return None;
            }
        }
        self.pending = None;
        self.route = Route::Call(wanted.clone());
        Some(self.route.clone())
    }
}

/// The device named `wanted`, only when no other device has the same name:
/// the name is all cpal tells, and two identical headsets would be mixed up.
pub fn unique_named<D>(devices: Vec<(String, D)>, wanted: &str) -> Option<D> {
    let mut found = None;
    for (name, device) in devices {
        if name == wanted {
            if found.is_some() {
                return None;
            }
            found = Some(device);
        }
    }
    found
}

/// The processes of Windows by id, for `app_of`.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub parent: u32,
    /// The lower-cased file name of the executable.
    pub exe: String,
}

/// The call app a process belongs to: its own executable when `calls::mic_app`
/// knows it, else the nearest ancestor that is. A call app's audio runs in
/// helper processes (a Chromium utility process, the web view of Teams).
#[cfg(any(target_os = "windows", test))]
pub fn app_of(pid: u32, processes: &std::collections::BTreeMap<u32, Proc>) -> Option<String> {
    const MAX_HOPS: usize = 6;
    let mut current = pid;
    for _ in 0..MAX_HOPS {
        let process = processes.get(&current)?;
        if mic_app(&process.exe).is_some() {
            return Some(process.exe.clone());
        }
        if process.parent == 0 || process.parent == current {
            return None;
        }
        current = process.parent;
    }
    None
}

/// The input devices the processes on the microphone read from now, `None`
/// where the system does not tell (before macOS 14, Intel Macs, other
/// systems, or a read that failed).
pub fn inputs_now() -> Option<Vec<ProcessInput>> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        macos::inputs()
    }
    #[cfg(target_os = "windows")]
    {
        super::call_input_windows::inputs()
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        target_os = "windows"
    )))]
    None
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos {
    use super::{parse_inputs, ProcessInput};

    extern "C" {
        /// Writes "pid<TAB>bundle id<TAB>device name" lines into `buffer`
        /// when they fit, and returns their length, or -1 before macOS 14.
        fn murmur_mic_user_inputs(buffer: *mut std::ffi::c_char, capacity: i32) -> i32;
    }

    pub fn inputs() -> Option<Vec<ProcessInput>> {
        let mut buffer = vec![0u8; 4096];
        loop {
            let capacity = i32::try_from(buffer.len()).ok()?;
            // SAFETY: the bridge writes at most `capacity` bytes into the
            // buffer, which outlives the call.
            let len = unsafe { murmur_mic_user_inputs(buffer.as_mut_ptr().cast(), capacity) };
            let len = usize::try_from(len).ok()?;
            if len < buffer.len() {
                return Some(parse_inputs(&String::from_utf8_lossy(&buffer[..len])));
            }
            buffer.resize(len + 1, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn user(app_id: &str, device: &str) -> ProcessInput {
        ProcessInput {
            app_id: app_id.to_string(),
            device: device.to_string(),
        }
    }

    fn names(devices: &[&str]) -> Vec<String> {
        devices.iter().map(|device| device.to_string()).collect()
    }

    #[test]
    fn the_list_the_mac_bridge_writes_is_read_per_process_and_device() {
        let list = "501\tus.zoom.xos\tUSB Headset\n77\t\tBuilt-in\nnope\n9\tcom.hnc.Discord\t\n\
                    12\tcom.google.Chrome.helper\tBuilt-in\n";
        assert_eq!(
            parse_inputs(list),
            [
                user("us.zoom.xos", "USB Headset"),
                user("com.google.Chrome.helper", "Built-in"),
            ]
        );
    }

    #[test]
    fn only_call_apps_and_browsers_say_which_device_a_call_uses() {
        let users = [
            user("com.example.dictation", "Built-in"),
            user("us.zoom.xos", "USB Headset"),
        ];
        assert_eq!(call_devices(&users), ["USB Headset"]);
        assert_eq!(call_devices(&users[..1]), Vec::<String>::new());
    }

    #[test]
    fn a_call_app_comes_before_a_browser_and_each_device_is_listed_once() {
        let users = [
            user("com.google.Chrome.helper", "Built-in"),
            user("com.microsoft.teams2", "AirPods"),
            user("com.microsoft.teams2.helper", "AirPods"),
            user("chrome.exe", "Built-in"),
            user("ms-teams.exe", "Headset"),
        ];
        assert_eq!(call_devices(&users), ["AirPods", "Headset", "Built-in"]);
    }

    fn look<'a>(calls: &'a [String], open: &'a str, explicit: bool, quiet: u64) -> Look<'a> {
        Look {
            calls,
            open: Some(open),
            explicit,
            quiet: Duration::from_secs(quiet),
        }
    }

    #[test]
    fn the_default_follows_the_device_the_call_app_uses() {
        let mut router = Router::default();
        let calls = names(&["Headset"]);
        let t = Instant::now();
        assert_eq!(router.start(&calls, false), &Route::Call("Headset".into()));
        assert_eq!(router.start(&[], false), &Route::Settings);
        assert_eq!(router.start(&calls, false), &Route::Call("Headset".into()));
        // Open on it already: nothing to do.
        assert_eq!(router.observe(t, &look(&calls, "Headset", false, 0)), None);
    }

    #[test]
    fn a_call_on_the_open_default_device_changes_nothing() {
        let mut router = Router::default();
        let calls = names(&["Built-in"]);
        let t = Instant::now();
        router.start(&[], false);
        for second in 0..10 {
            let at = t + Duration::from_secs(second);
            assert_eq!(
                router.observe(at, &look(&calls, "Built-in", false, 0)),
                None
            );
        }
        assert_eq!(router.route(), &Route::Settings);
    }

    #[test]
    fn a_call_that_starts_on_another_device_moves_the_track_after_a_short_wait() {
        let mut router = Router::default();
        router.start(&[], false);
        let calls = names(&["AirPods"]);
        let t = Instant::now();
        assert_eq!(router.observe(t, &look(&calls, "Built-in", false, 0)), None);
        assert_eq!(
            router.observe(
                t + SETTLE - Duration::from_millis(1),
                &look(&calls, "Built-in", false, 0)
            ),
            None
        );
        assert_eq!(
            router.observe(t + SETTLE, &look(&calls, "Built-in", false, 0)),
            Some(Route::Call("AirPods".into()))
        );
        assert_eq!(router.route(), &Route::Call("AirPods".into()));
    }

    #[test]
    fn a_device_the_call_leaves_within_the_wait_never_moves_the_track() {
        let mut router = Router::default();
        router.start(&[], false);
        let t = Instant::now();
        let flicker = names(&["AirPods"]);
        assert_eq!(
            router.observe(t, &look(&flicker, "Built-in", false, 0)),
            None
        );
        // The call is back on the open device a second later.
        let back = names(&["Built-in"]);
        let second = Duration::from_secs(1);
        assert_eq!(
            router.observe(t + second, &look(&back, "Built-in", false, 0)),
            None
        );
        // A later flicker starts its wait again.
        assert_eq!(
            router.observe(t + 2 * second, &look(&flicker, "Built-in", false, 0)),
            None
        );
        assert_eq!(
            router.observe(t + 3 * second, &look(&flicker, "Built-in", false, 0)),
            None
        );
        assert_eq!(router.route(), &Route::Settings);
    }

    #[test]
    fn the_track_follows_the_call_app_when_it_switches_devices_mid_call() {
        let mut router = Router::default();
        let first = names(&["Headset"]);
        router.start(&first, false);
        let t = Instant::now();
        assert_eq!(router.observe(t, &look(&first, "Headset", false, 0)), None);

        let second = names(&["AirPods"]);
        assert_eq!(router.observe(t, &look(&second, "Headset", false, 0)), None);
        assert_eq!(
            router.observe(t + SETTLE, &look(&second, "Headset", false, 0)),
            Some(Route::Call("AirPods".into()))
        );
    }

    #[test]
    fn a_call_app_that_leaves_the_microphone_leaves_the_track_where_it_is() {
        let mut router = Router::default();
        let calls = names(&["Headset"]);
        router.start(&calls, false);
        let t = Instant::now();
        for second in 0..30 {
            let at = t + Duration::from_secs(second);
            assert_eq!(router.observe(at, &look(&[], "Headset", false, 0)), None);
        }
        assert_eq!(router.route(), &Route::Call("Headset".into()));
    }

    #[test]
    fn a_microphone_chosen_in_settings_is_kept_while_it_is_alive() {
        let mut router = Router::default();
        let calls = names(&["AirPods"]);
        assert_eq!(router.start(&calls, true), &Route::Settings);
        let t = Instant::now();
        for second in 0..60 {
            let at = t + Duration::from_secs(second);
            let quiet = look(&calls, "USB Mic", true, 9);
            assert_eq!(router.observe(at, &quiet), None);
        }
    }

    #[test]
    fn a_microphone_chosen_in_settings_that_stays_silent_gives_way_to_the_call() {
        let mut router = Router::default();
        let calls = names(&["AirPods"]);
        router.start(&calls, true);
        let t = Instant::now();
        let silent = look(&calls, "USB Mic", true, SILENT_AFTER.as_secs());
        assert_eq!(router.observe(t, &silent), None);
        assert_eq!(
            router.observe(t + SETTLE, &silent),
            Some(Route::Call("AirPods".into()))
        );
        // It stays there: the track is on the call's device now.
        let after = look(&calls, "AirPods", false, 0);
        assert_eq!(router.observe(t + 2 * SETTLE, &after), None);
        assert_eq!(router.route(), &Route::Call("AirPods".into()));
    }

    #[test]
    fn a_silent_chosen_microphone_stays_when_the_call_uses_it_or_no_call_runs() {
        let mut router = Router::default();
        let t = Instant::now();
        let same = names(&["USB Mic"]);
        for second in 0..5 {
            let at = t + Duration::from_secs(second);
            assert_eq!(router.observe(at, &look(&same, "USB Mic", true, 60)), None);
            assert_eq!(router.observe(at, &look(&[], "USB Mic", true, 60)), None);
        }
    }

    #[test]
    fn devices_are_matched_by_name_only_when_the_name_is_unique() {
        let devices = |names: &[&'static str]| -> Vec<(String, &'static str)> {
            names.iter().map(|name| (name.to_string(), *name)).collect()
        };
        assert_eq!(
            unique_named(devices(&["Built-in", "AirPods"]), "AirPods"),
            Some("AirPods")
        );
        assert_eq!(unique_named(devices(&["Built-in"]), "AirPods"), None);
        assert_eq!(
            unique_named(devices(&["Headset", "Built-in", "Headset"]), "Headset"),
            None,
            "two devices with one name cannot be told apart"
        );
    }

    fn processes(list: &[(u32, u32, &str)]) -> BTreeMap<u32, Proc> {
        list.iter()
            .map(|&(pid, parent, exe)| {
                (
                    pid,
                    Proc {
                        parent,
                        exe: exe.to_string(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_helper_process_belongs_to_the_call_app_that_started_it() {
        let table = processes(&[
            (1, 0, "explorer.exe"),
            (10, 1, "ms-teams.exe"),
            (11, 10, "msedgewebview2.exe"),
            (12, 11, "msedgewebview2.exe"),
            (20, 1, "chrome.exe"),
            (21, 20, "chrome.exe"),
            (30, 1, "notepad.exe"),
        ]);
        assert_eq!(app_of(12, &table).as_deref(), Some("ms-teams.exe"));
        assert_eq!(app_of(10, &table).as_deref(), Some("ms-teams.exe"));
        assert_eq!(app_of(21, &table).as_deref(), Some("chrome.exe"));
        assert_eq!(app_of(30, &table), None);
        assert_eq!(app_of(999, &table), None);
    }

    #[test]
    fn a_loop_or_a_long_chain_of_processes_ends_without_an_app() {
        let looped = processes(&[(5, 6, "a.exe"), (6, 5, "b.exe")]);
        assert_eq!(app_of(5, &looped), None);
        let own_parent = processes(&[(7, 7, "c.exe")]);
        assert_eq!(app_of(7, &own_parent), None);
        let long = processes(&[
            (1, 2, "a.exe"),
            (2, 3, "a.exe"),
            (3, 4, "a.exe"),
            (4, 5, "a.exe"),
            (5, 6, "a.exe"),
            (6, 7, "a.exe"),
            (7, 8, "zoom.exe"),
            (8, 0, "zoom.exe"),
        ]);
        assert_eq!(app_of(1, &long), None);
        assert_eq!(app_of(7, &long).as_deref(), Some("zoom.exe"));
    }
}
