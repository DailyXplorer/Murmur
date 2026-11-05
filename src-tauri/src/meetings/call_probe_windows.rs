//! What Windows tells about calls, for `call_watch.rs`: which apps hold the
//! microphone, read from the microphone consent store, and which meeting a
//! browser shows, read from its window titles and its address bar. `calls.rs`
//! decides what that means. The decisions and parsers are plain functions,
//! tested everywhere; only the reads of the system sit behind `windows`.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use super::calls::{Browser, MicApp, MicUser, WINDOWS_EXECUTABLES};

/// The consent store's microphone key, under `HKEY_CURRENT_USER`. Windows
/// writes when each app opened and closed the microphone under it: a
/// subkey per packaged app (its package family name), and per desktop app,
/// under `NonPackaged`, one named by the executable's path with `#` for `\`.
#[cfg(target_os = "windows")]
const MICROPHONE_CONSENT: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// How often a browser's address bar is read while the browser holds the
/// microphone and no window title showed a meeting: the read walks the
/// browser's accessibility tree, which a title check does not.
const ADDRESS_EVERY: Duration = Duration::from_secs(5);

/// One app of the consent store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    /// The executable's lower-cased file name of a desktop app, or the
    /// package family name of a packaged one: what `calls::mic_app` knows.
    pub app_id: String,
    /// Whether the app is a desktop app, whose executable can be looked for
    /// among the running processes.
    pub desktop: bool,
    /// `LastUsedTimeStart` and `LastUsedTimeStop`, FILETIMEs, 0 when absent.
    pub last_start: u64,
    pub last_stop: u64,
}

impl Consent {
    /// Windows sets Start when the app opens the microphone and Stop when it
    /// closes it, so a Start with no Stop since is an open microphone.
    pub fn on_the_microphone(&self) -> bool {
        self.last_start != 0 && self.last_stop == 0
    }
}

/// The app id of a desktop app's consent key, `C:#Apps#Zoom#bin#Zoom.exe`:
/// the file name, lower-cased.
pub fn desktop_app_id(key_name: &str) -> Option<String> {
    let name = key_name.rsplit('#').next()?.trim();
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

/// The apps on the microphone now, other than Murmur (`own_executable`, a
/// lower-cased file name): its own dictation holds the microphone too.
///
/// Windows leaves Stop at 0 when an app dies with the microphone open, so a
/// desktop app counts only while `running` lists its executable. `running` is
/// asked once, and only when a desktop app is on the microphone; `None`
/// (the process list could not be read) trusts the store.
pub fn mic_users(
    consents: &[Consent],
    own_executable: &str,
    running: impl FnOnce() -> Option<BTreeSet<String>>,
) -> Vec<MicUser> {
    let open: Vec<&Consent> = consents
        .iter()
        .filter(|consent| consent.on_the_microphone())
        .filter(|consent| !(consent.desktop && consent.app_id == own_executable))
        .collect();
    let alive = if open.iter().any(|consent| consent.desktop) {
        running()
    } else {
        None
    };
    open.into_iter()
        .filter(|consent| {
            !consent.desktop
                || alive
                    .as_ref()
                    .is_none_or(|alive| alive.contains(&consent.app_id))
        })
        .map(|consent| MicUser {
            pid: 0,
            bundle_id: consent.app_id.clone(),
        })
        .collect()
}

/// The executables of `browser`, whose windows show its pages.
pub fn browser_executables(browser: Browser) -> Vec<&'static str> {
    WINDOWS_EXECUTABLES
        .iter()
        .filter(|(_, app)| *app == MicApp::Browser(browser))
        .map(|&(executable, _)| executable)
        .collect()
}

/// An address bar's text as a URL for `calls::meeting_url`. Browsers hide
/// the scheme of the address they show; text with a space is a search being
/// typed, not an address.
pub fn address_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    if text.contains("://") {
        Some(text.to_string())
    } else {
        Some(format!("https://{text}"))
    }
}

/// When each browser's address bar was last read.
#[derive(Debug, Default)]
pub struct AddressReads {
    last: BTreeMap<Browser, Instant>,
}

impl AddressReads {
    /// Whether `browser`'s address bar may be read at `now`: the first time,
    /// then every `ADDRESS_EVERY`. Asking counts as reading.
    pub fn due(&mut self, browser: Browser, now: Instant) -> bool {
        let due = self
            .last
            .get(&browser)
            .is_none_or(|last| now.saturating_duration_since(*last) >= ADDRESS_EVERY);
        if due {
            self.last.insert(browser, now);
        }
        due
    }
}

#[cfg(target_os = "windows")]
pub use system::{mic_users_now, supported, PageReader};

#[cfg(target_os = "windows")]
mod system {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io;
    use std::time::{Duration, Instant};

    use windows::core::BOOL;
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::UI::Accessibility::{UIA_DocumentControlTypeId, UIA_EditControlTypeId};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    use super::{
        address_url, browser_executables, desktop_app_id, mic_users, AddressReads, Consent,
        MICROPHONE_CONSENT,
    };
    use crate::meetings::calls::{self, Browser, CallApp, MicUser};
    use crate::uia::{Automation, Element};

    /// Window titles are read up to this many UTF-16 units: a meeting's title
    /// starts with its name.
    const TITLE_UNITS: usize = 512;
    /// The most browser windows whose address bar one read looks at.
    const MAX_WINDOWS: usize = 4;
    /// How deep, how wide and how long one address bar search goes in a
    /// browser's accessibility tree, outside the page itself.
    const MAX_DEPTH: usize = 8;
    const MAX_ELEMENTS: usize = 400;
    const SEARCH_BUDGET: Duration = Duration::from_millis(750);

    /// The consent store's apps under `microphone`, which a test points at a
    /// key of its own.
    pub(super) fn read_consents(microphone: &RegKey) -> Vec<Consent> {
        let mut consents = Vec::new();
        for name in microphone.enum_keys().filter_map(Result::ok) {
            if name.eq_ignore_ascii_case("NonPackaged") {
                let Ok(desktop) = microphone.open_subkey(&name) else {
                    continue;
                };
                for key_name in desktop.enum_keys().filter_map(Result::ok) {
                    let Some(app_id) = desktop_app_id(&key_name) else {
                        continue;
                    };
                    consents.extend(consent_of(desktop.open_subkey(&key_name), app_id, true));
                }
            } else {
                consents.extend(consent_of(microphone.open_subkey(&name), name, false));
            }
        }
        consents
    }

    fn consent_of(key: io::Result<RegKey>, app_id: String, desktop: bool) -> Option<Consent> {
        let key = key.ok()?;
        Some(Consent {
            app_id,
            desktop,
            last_start: key.get_value("LastUsedTimeStart").unwrap_or(0),
            last_stop: key.get_value("LastUsedTimeStop").unwrap_or(0),
        })
    }

    /// The lower-cased file name of the executables running now, `None` when
    /// Windows would not list them.
    fn running_executables() -> Option<BTreeSet<String>> {
        // SAFETY: plain call; the handle is closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
        let mut names = BTreeSet::new();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: `entry` is a live local with its size set, and each call
        // writes only into it.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while more {
            let length = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            names.insert(String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase());
            more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        // SAFETY: the handle came from CreateToolhelp32Snapshot.
        let _ = unsafe { CloseHandle(snapshot) };
        Some(names)
    }

    fn own_executable() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_lowercase())
            })
            .unwrap_or_default()
    }

    fn microphone_key() -> io::Result<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(MICROPHONE_CONSENT)
    }

    /// Whether this Windows keeps the consent store: from Windows 10 1903.
    pub fn supported() -> bool {
        microphone_key().is_ok()
    }

    /// The apps on the microphone now, `None` when the store cannot be read.
    pub fn mic_users_now() -> Option<Vec<MicUser>> {
        let microphone = microphone_key().ok()?;
        Some(mic_users(
            &read_consents(&microphone),
            &own_executable(),
            running_executables,
        ))
    }

    /// A visible top-level window with a title.
    struct TopLevel {
        window: isize,
        pid: u32,
        title: String,
    }

    unsafe extern "system" fn collect_window(window: HWND, collected: LPARAM) -> BOOL {
        // SAFETY: `collected` is the vector `top_level_windows` lent for the
        // call, alive until EnumWindows returns.
        let windows = unsafe { &mut *(collected.0 as *mut Vec<TopLevel>) };
        // SAFETY: `window` is the handle EnumWindows passed; the buffer is a
        // live local.
        unsafe {
            if IsWindowVisible(window).as_bool() {
                let mut title = [0u16; TITLE_UNITS];
                let length = usize::try_from(GetWindowTextW(window, &mut title)).unwrap_or(0);
                if length > 0 {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(window, Some(&mut pid));
                    windows.push(TopLevel {
                        window: window.0 as isize,
                        pid,
                        title: String::from_utf16_lossy(&title[..length.min(TITLE_UNITS)]),
                    });
                }
            }
        }
        BOOL(1)
    }

    fn top_level_windows() -> Vec<TopLevel> {
        let mut windows: Vec<TopLevel> = Vec::new();
        // SAFETY: the pointer is a live local, used only inside the call.
        let listed = unsafe {
            EnumWindows(
                Some(collect_window),
                LPARAM(std::ptr::addr_of_mut!(windows) as isize),
            )
        };
        if listed.is_err() {
            windows.clear();
        }
        windows
    }

    /// The windows of `browser`, found by the executable that owns each.
    fn browser_windows(browser: Browser) -> Vec<TopLevel> {
        let executables = browser_executables(browser);
        let mut owned: BTreeMap<u32, bool> = BTreeMap::new();
        top_level_windows()
            .into_iter()
            .filter(|window| {
                *owned.entry(window.pid).or_insert_with(|| {
                    crate::foreground::process_image_path(window.pid)
                        .and_then(|path| crate::foreground::executable_name(&path))
                        .is_some_and(|name| executables.contains(&name.as_str()))
                })
            })
            .collect()
    }

    /// The address a browser window shows, from the edit field of its toolbar
    /// in the accessibility tree. The page itself is never entered: it is
    /// large, and only the toolbar is wanted.
    fn address_bar(window: isize) -> Option<String> {
        let automation = Automation::new().ok()?;
        let deadline = Instant::now() + SEARCH_BUDGET;
        let mut level: Vec<Element> = vec![automation.window(window)?];
        let mut seen = 0usize;
        for _ in 0..MAX_DEPTH {
            let mut next = Vec::new();
            for element in &level {
                if Instant::now() >= deadline {
                    return None;
                }
                for child in automation.children(element) {
                    seen += 1;
                    if seen > MAX_ELEMENTS {
                        return None;
                    }
                    match child.control_type() {
                        Some(kind) if kind == UIA_EditControlTypeId => {
                            if let Some(url) = child.value().and_then(|text| address_url(&text)) {
                                return Some(url);
                            }
                        }
                        Some(kind) if kind == UIA_DocumentControlTypeId => {}
                        _ => next.push(child),
                    }
                }
            }
            if next.is_empty() {
                return None;
            }
            level = next;
        }
        None
    }

    /// Which meeting a browser shows, for `call_watch.rs`.
    #[derive(Default)]
    pub struct PageReader {
        reads: AddressReads,
    }

    impl PageReader {
        /// The meeting `browser` shows: a window title, which is one call to
        /// Windows, or else, once in a while, the address bar of its windows.
        /// Nothing is logged.
        pub fn browser_call(&mut self, browser: Browser, now: Instant) -> Option<CallApp> {
            let windows = browser_windows(browser);
            let titles: Vec<String> = windows.iter().map(|window| window.title.clone()).collect();
            calls::meeting_page(&titles, || {
                if !self.reads.due(browser, now) {
                    return None;
                }
                windows
                    .iter()
                    .take(MAX_WINDOWS)
                    .filter_map(|window| address_bar(window.window))
                    .find(|url| calls::meeting_url(url).is_some())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::calls::{calls_on_mic, meeting_url, mic_app, CallApp};

    fn consent(app_id: &str, desktop: bool, start: u64, stop: u64) -> Consent {
        Consent {
            app_id: app_id.to_string(),
            desktop,
            last_start: start,
            last_stop: stop,
        }
    }

    fn ids(users: &[MicUser]) -> Vec<&str> {
        users.iter().map(|user| user.bundle_id.as_str()).collect()
    }

    #[test]
    fn a_desktop_app_is_named_by_the_file_name_of_its_key() {
        assert_eq!(
            desktop_app_id("C:#Users#someone#AppData#Roaming#Zoom#bin#Zoom.exe").as_deref(),
            Some("zoom.exe")
        );
        assert_eq!(
            desktop_app_id("C:#Program Files#Google#Chrome#Application#chrome.exe").as_deref(),
            Some("chrome.exe")
        );
        assert_eq!(desktop_app_id("Slack.EXE").as_deref(), Some("slack.exe"));
        assert_eq!(desktop_app_id("C:#folder#"), None);
        assert_eq!(desktop_app_id(""), None);
    }

    #[test]
    fn an_app_is_on_the_microphone_between_a_start_and_its_stop() {
        assert!(consent("zoom.exe", true, 133_000, 0).on_the_microphone());
        assert!(!consent("zoom.exe", true, 133_000, 134_000).on_the_microphone());
        assert!(
            !consent("zoom.exe", true, 0, 0).on_the_microphone(),
            "never used"
        );
        assert!(!consent("zoom.exe", true, 0, 5).on_the_microphone());
    }

    #[test]
    fn only_the_apps_holding_the_microphone_are_listed_and_never_murmur() {
        let consents = [
            consent("zoom.exe", true, 10, 0),
            consent("slack.exe", true, 10, 20),
            consent("murmur.exe", true, 10, 0),
            consent("MSTeams_8wekyb3d8bbwe", false, 10, 0),
            consent("Contoso.Recorder_abc123", false, 0, 0),
        ];
        let running = || {
            Some(BTreeSet::from([
                "zoom.exe".to_string(),
                "murmur.exe".to_string(),
            ]))
        };
        let users = mic_users(&consents, "murmur.exe", running);
        assert_eq!(ids(&users), ["zoom.exe", "MSTeams_8wekyb3d8bbwe"]);
        assert!(users.iter().all(|user| user.pid == 0));
    }

    #[test]
    fn a_desktop_app_that_died_with_the_microphone_open_is_not_a_call() {
        let consents = [consent("zoom.exe", true, 10, 0)];
        let users = mic_users(&consents, "murmur.exe", || Some(BTreeSet::new()));
        assert!(users.is_empty());

        let users = mic_users(&consents, "murmur.exe", || None);
        assert_eq!(ids(&users), ["zoom.exe"], "the store is trusted unaided");
    }

    #[test]
    fn the_process_list_is_read_only_when_a_desktop_app_is_on_the_microphone() {
        let consents = [
            consent("MSTeams_8wekyb3d8bbwe", false, 10, 0),
            consent("zoom.exe", true, 10, 20),
        ];
        let users = mic_users(&consents, "murmur.exe", || {
            panic!("the processes were listed for nothing")
        });
        assert_eq!(ids(&users), ["MSTeams_8wekyb3d8bbwe"]);
    }

    #[test]
    fn every_app_the_table_lists_is_found_from_what_the_store_names() {
        let known = [
            ("zoom.exe", CallApp::Zoom),
            ("ms-teams.exe", CallApp::Teams),
            ("Teams.exe", CallApp::Teams),
            ("MSTeams_8wekyb3d8bbwe", CallApp::Teams),
            ("slack.exe", CallApp::Slack),
            ("CiscoCollabHost.exe", CallApp::Webex),
            ("webexmta.exe", CallApp::Webex),
            ("atmgr.exe", CallApp::Webex),
            ("Discord.exe", CallApp::Discord),
        ];
        for (app_id, call) in known {
            assert_eq!(mic_app(app_id), Some(MicApp::Call(call)), "{app_id}");
        }
        for (executable, browser) in [
            ("chrome.exe", Browser::Chrome),
            ("msedge.exe", Browser::Edge),
            ("brave.exe", Browser::Brave),
            ("firefox.exe", Browser::Firefox),
            ("vivaldi.exe", Browser::Vivaldi),
            ("opera.exe", Browser::Opera),
            ("chromium.exe", Browser::Chromium),
            ("arc.exe", Browser::Arc),
            ("helium.exe", Browser::Helium),
            ("comet.exe", Browser::Comet),
        ] {
            assert_eq!(
                mic_app(executable),
                Some(MicApp::Browser(browser)),
                "{executable}"
            );
            assert_eq!(browser_executables(browser), [executable]);
        }
    }

    #[test]
    fn other_apps_and_look_alike_names_are_no_call() {
        for app_id in [
            "notzoom.exe",
            "zoom.exe.bak",
            "Contoso.Recorder_abc123",
            "MSTeamsHelper_abc123",
            "chrome",
            "",
        ] {
            assert_eq!(mic_app(app_id), None, "{app_id}");
        }
    }

    #[test]
    fn a_call_app_on_the_microphone_is_a_call_and_a_browser_needs_its_page() {
        let users = [
            MicUser {
                pid: 0,
                bundle_id: "zoom.exe".into(),
            },
            MicUser {
                pid: 0,
                bundle_id: "chrome.exe".into(),
            },
        ];
        let mut cached = BTreeMap::new();
        let found = calls_on_mic(&users, 4242, &mut cached, |browser| {
            assert_eq!(browser, Browser::Chrome);
            None
        });
        assert_eq!(found, [CallApp::Zoom]);

        let found = calls_on_mic(&users, 4242, &mut cached, |_| Some(CallApp::GoogleMeet));
        assert_eq!(found, [CallApp::Zoom, CallApp::GoogleMeet]);
    }

    #[test]
    fn an_address_bar_shows_the_url_with_or_without_its_scheme() {
        assert_eq!(
            address_url("meet.google.com/abc-defg-hij").as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(
            address_url(" https://teams.microsoft.com/v2/ ").as_deref(),
            Some("https://teams.microsoft.com/v2/")
        );
        assert_eq!(address_url("how to cook rice"), None);
        assert_eq!(address_url("   "), None);
        for (typed, call) in [
            ("meet.google.com/abc-defg-hij", CallApp::GoogleMeet),
            ("teams.microsoft.com/v2/", CallApp::Teams),
            ("app.zoom.us/wc/123/join", CallApp::Zoom),
            ("company.webex.com/meet/someone", CallApp::Webex),
            ("discord.com/channels/1/2", CallApp::Discord),
        ] {
            let url = address_url(typed).unwrap();
            assert_eq!(meeting_url(&url), Some(call), "{typed}");
        }
        let home = address_url("meet.google.com").unwrap();
        assert_eq!(meeting_url(&home), None, "the Meet home page is no call");
    }

    #[test]
    fn a_meet_window_title_names_the_call_whatever_the_browser_adds() {
        use crate::meetings::calls::meeting_page;
        let titles = vec![
            "Inbox - Mail".to_string(),
            "Meet - abc-defg-hij - Google Chrome".to_string(),
        ];
        assert_eq!(
            meeting_page(&titles, || panic!("the address was read for a title")),
            Some(CallApp::GoogleMeet)
        );
        assert_eq!(
            meeting_page(&["Meeting notes - Google Chrome".to_string()], || None),
            None
        );
    }

    #[test]
    fn an_address_bar_is_read_at_most_every_five_seconds_per_browser() {
        let start = Instant::now();
        let mut reads = AddressReads::default();
        assert!(reads.due(Browser::Chrome, start));
        assert!(!reads.due(Browser::Chrome, start + Duration::from_secs(1)));
        assert!(!reads.due(Browser::Chrome, start + Duration::from_secs(4)));
        assert!(reads.due(Browser::Edge, start + Duration::from_secs(1)));
        assert!(reads.due(Browser::Chrome, start + Duration::from_secs(5)));
        assert!(!reads.due(Browser::Chrome, start + Duration::from_secs(9)));
        assert!(reads.due(Browser::Chrome, start + Duration::from_secs(10)));
    }

    #[cfg(target_os = "windows")]
    mod registry {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        /// A throwaway key of the test's own under HKEY_CURRENT_USER.
        struct Fake {
            path: String,
            root: RegKey,
        }

        impl Fake {
            fn new(name: &str) -> Self {
                let path = format!(r"Software\MurmurTests\{}-{}", name, std::process::id());
                let (root, _) = RegKey::predef(HKEY_CURRENT_USER)
                    .create_subkey(&path)
                    .expect("create the throwaway key");
                Self { path, root }
            }

            fn app(&self, subkey: &str, start: Option<u64>, stop: Option<u64>) {
                let (key, _) = self.root.create_subkey(subkey).expect("create the app key");
                if let Some(start) = start {
                    key.set_value("LastUsedTimeStart", &start).unwrap();
                }
                if let Some(stop) = stop {
                    key.set_value("LastUsedTimeStop", &stop).unwrap();
                }
            }
        }

        impl Drop for Fake {
            fn drop(&mut self) {
                let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.path);
            }
        }

        #[test]
        fn the_consent_store_is_read_for_packaged_and_desktop_apps() {
            let fake = Fake::new("consents");
            fake.app("MSTeams_8wekyb3d8bbwe", Some(10), Some(0));
            fake.app("Contoso.Recorder_abc123", Some(10), Some(20));
            fake.app(r"NonPackaged\C:#Apps#Zoom#bin#Zoom.exe", Some(10), Some(0));
            fake.app(r"NonPackaged\C:#Apps#Slack#slack.exe", Some(10), Some(20));
            fake.app(r"NonPackaged\C:#Apps#Never#never.exe", None, None);

            let mut consents =
                crate::meetings::call_probe_windows::system::read_consents(&fake.root);
            consents.sort_by(|a, b| a.app_id.cmp(&b.app_id));
            let summary: Vec<(&str, bool, bool)> = consents
                .iter()
                .map(|c| (c.app_id.as_str(), c.desktop, c.on_the_microphone()))
                .collect();
            assert_eq!(
                summary,
                [
                    ("Contoso.Recorder_abc123", false, false),
                    ("MSTeams_8wekyb3d8bbwe", false, true),
                    ("never.exe", true, false),
                    ("slack.exe", true, false),
                    ("zoom.exe", true, true),
                ]
            );
        }

        #[test]
        fn a_missing_microphone_key_lists_nothing() {
            let fake = Fake::new("empty");
            assert!(
                crate::meetings::call_probe_windows::system::read_consents(&fake.root).is_empty()
            );
        }
    }
}
