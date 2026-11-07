//! Which calls are on, from the apps holding the microphone, when to offer
//! to record one, and when to offer to stop a meeting whose call ended.
//! Pure: `call_watch.rs` polls the system and shows what this decides.
//! Nothing here starts or stops a meeting.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long an app holds the microphone before the prompt: a device check
/// or a short voice message is over sooner.
pub const PROMPT_AFTER: Duration = Duration::from_secs(3);
/// Off the microphone this long, the call has ended and its prompt closes.
/// Slack and Zoom drop the microphone for a moment on mute or a device
/// change.
pub const ENDED_AFTER: Duration = Duration::from_secs(15);
/// Off this long, the call is over: the app's next use of the microphone is
/// a new call, which "Not now" no longer mutes.
pub const CALL_OVER_AFTER: Duration = Duration::from_secs(30);
/// Off the microphone this long while a meeting records, the call recorded
/// has ended: Murmur offers to stop. Coming back hides the offer, so a mute
/// that drops the microphone costs a prompt, never a recording.
pub const ENDED_PROMPT_AFTER: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallApp {
    Zoom,
    GoogleMeet,
    Teams,
    Slack,
    FaceTime,
    Webex,
    Discord,
}

impl CallApp {
    /// Brand names, the same in every language.
    pub fn name(self) -> &'static str {
        match self {
            Self::Zoom => "Zoom",
            Self::GoogleMeet => "Google Meet",
            Self::Teams => "Microsoft Teams",
            Self::Slack => "Slack",
            Self::FaceTime => "FaceTime",
            Self::Webex => "Webex",
            Self::Discord => "Discord",
        }
    }
}

/// A browser edition: its windows and cached call must not belong to a
/// different edition running alongside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Browser {
    Safari,
    Chrome,
    ChromeBeta,
    ChromeCanary,
    Arc,
    Edge,
    EdgeBeta,
    Brave,
    Helium,
    Vivaldi,
    Opera,
    Chromium,
    Dia,
    Comet,
    Firefox,
}

impl Browser {
    /// The app whose windows show the page. The process on the microphone is
    /// often one of its helpers, or WebKit's for Safari. macOS only: Windows
    /// finds a browser's windows by its executables, `WINDOWS_EXECUTABLES`.
    #[cfg(any(target_os = "macos", test))]
    pub fn bundle_id(self) -> &'static str {
        match self {
            Self::Safari => "com.apple.Safari",
            Self::Chrome => "com.google.Chrome",
            Self::ChromeBeta => "com.google.Chrome.beta",
            Self::ChromeCanary => "com.google.Chrome.canary",
            Self::Arc => "company.thebrowser.Browser",
            Self::Edge => "com.microsoft.edgemac",
            Self::EdgeBeta => "com.microsoft.edgemac.Beta",
            Self::Brave => "com.brave.Browser",
            Self::Helium => "net.imput.helium",
            Self::Vivaldi => "com.vivaldi.Vivaldi",
            Self::Opera => "com.operasoftware.Opera",
            Self::Chromium => "org.chromium.Chromium",
            Self::Dia => "company.thebrowser.dia",
            Self::Comet => "ai.perplexity.comet",
            Self::Firefox => "org.mozilla.firefox",
        }
    }

    /// Whether the address of its front page is read as well as the window
    /// titles. macOS only: Windows reads the address bar of the browsers
    /// it can, in `call_probe_windows.rs`.
    #[cfg(any(target_os = "macos", test))]
    pub fn reads_url(self) -> bool {
        matches!(
            self,
            Self::Safari | Self::Chrome | Self::ChromeBeta | Self::ChromeCanary
        )
    }
}

/// What a process on the microphone is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicApp {
    Call(CallApp),
    /// A call only on a meeting page.
    Browser(Browser),
}

const MIC_APPS: [(&str, MicApp); 27] = [
    ("us.zoom.xos", MicApp::Call(CallApp::Zoom)),
    ("com.microsoft.teams2", MicApp::Call(CallApp::Teams)),
    ("com.microsoft.teams", MicApp::Call(CallApp::Teams)),
    ("com.tinyspeck.slackmacgap", MicApp::Call(CallApp::Slack)),
    ("com.apple.FaceTime", MicApp::Call(CallApp::FaceTime)),
    // FaceTime's calls run in this daemon, not in the app.
    ("com.apple.avconferenced", MicApp::Call(CallApp::FaceTime)),
    ("Cisco-Systems.Spark", MicApp::Call(CallApp::Webex)),
    ("com.webex.meetingmanager", MicApp::Call(CallApp::Webex)),
    ("com.cisco.webexmeetingsapp", MicApp::Call(CallApp::Webex)),
    ("com.hnc.Discord", MicApp::Call(CallApp::Discord)),
    ("com.apple.Safari", MicApp::Browser(Browser::Safari)),
    // Safari's pages take the microphone in WebKit's media process.
    ("com.apple.WebKit.GPU", MicApp::Browser(Browser::Safari)),
    ("com.google.Chrome", MicApp::Browser(Browser::Chrome)),
    ("company.thebrowser.Browser", MicApp::Browser(Browser::Arc)),
    ("com.microsoft.edgemac", MicApp::Browser(Browser::Edge)),
    ("com.brave.Browser", MicApp::Browser(Browser::Brave)),
    ("net.imput.helium", MicApp::Browser(Browser::Helium)),
    ("com.vivaldi.Vivaldi", MicApp::Browser(Browser::Vivaldi)),
    ("com.operasoftware.Opera", MicApp::Browser(Browser::Opera)),
    ("org.chromium.Chromium", MicApp::Browser(Browser::Chromium)),
    ("company.thebrowser.dia", MicApp::Browser(Browser::Dia)),
    ("ai.perplexity.comet", MicApp::Browser(Browser::Comet)),
    ("org.mozilla.firefox", MicApp::Browser(Browser::Firefox)),
    // Firefox's pages run in this process.
    (
        "org.mozilla.plugincontainer",
        MicApp::Browser(Browser::Firefox),
    ),
    (
        "com.google.Chrome.beta",
        MicApp::Browser(Browser::ChromeBeta),
    ),
    (
        "com.google.Chrome.canary",
        MicApp::Browser(Browser::ChromeCanary),
    ),
    (
        "com.microsoft.edgemac.Beta",
        MicApp::Browser(Browser::EdgeBeta),
    ),
];

/// `bundle_id` is `known` or one of its helpers, such as
/// `com.google.Chrome.helper`.
fn same_app(bundle_id: &str, known: &str) -> bool {
    bundle_id.len() >= known.len()
        && bundle_id.is_char_boundary(known.len())
        && bundle_id[..known.len()].eq_ignore_ascii_case(known)
        && matches!(bundle_id.as_bytes().get(known.len()), None | Some(b'.'))
}

/// The desktop apps of Windows that hold a call, by the lower-cased file name
/// of their executable, which is how the microphone consent store names
/// them. Chrome's Beta and Canary share `chrome.exe`, and every Edge channel
/// `msedge.exe`, so they are one browser here.
pub const WINDOWS_EXECUTABLES: [(&str, MicApp); 19] = [
    ("zoom.exe", MicApp::Call(CallApp::Zoom)),
    ("ms-teams.exe", MicApp::Call(CallApp::Teams)),
    ("teams.exe", MicApp::Call(CallApp::Teams)),
    ("slack.exe", MicApp::Call(CallApp::Slack)),
    ("webex.exe", MicApp::Call(CallApp::Webex)),
    ("ciscocollabhost.exe", MicApp::Call(CallApp::Webex)),
    ("webexmta.exe", MicApp::Call(CallApp::Webex)),
    ("atmgr.exe", MicApp::Call(CallApp::Webex)),
    ("discord.exe", MicApp::Call(CallApp::Discord)),
    ("chrome.exe", MicApp::Browser(Browser::Chrome)),
    ("msedge.exe", MicApp::Browser(Browser::Edge)),
    ("brave.exe", MicApp::Browser(Browser::Brave)),
    ("firefox.exe", MicApp::Browser(Browser::Firefox)),
    ("vivaldi.exe", MicApp::Browser(Browser::Vivaldi)),
    ("opera.exe", MicApp::Browser(Browser::Opera)),
    ("chromium.exe", MicApp::Browser(Browser::Chromium)),
    ("arc.exe", MicApp::Browser(Browser::Arc)),
    ("helium.exe", MicApp::Browser(Browser::Helium)),
    ("comet.exe", MicApp::Browser(Browser::Comet)),
];

/// The packaged apps of Windows, by the lower-cased name that starts their
/// package family name (`MSTeams_8wekyb3d8bbwe`, the part before the
/// underscore). The consent store names a packaged app by its family.
pub const WINDOWS_PACKAGES: [(&str, MicApp); 4] = [
    ("msteams", MicApp::Call(CallApp::Teams)),
    ("microsoftteams", MicApp::Call(CallApp::Teams)),
    ("91750d7e.slack", MicApp::Call(CallApp::Slack)),
    ("mozilla.firefox", MicApp::Browser(Browser::Firefox)),
];

/// A Windows app id: an executable's file name, or a package family name.
fn windows_app(app_id: &str) -> Option<MicApp> {
    let app_id = app_id.to_ascii_lowercase();
    if let Some(&(_, app)) = WINDOWS_EXECUTABLES
        .iter()
        .find(|(known, _)| *known == app_id)
    {
        return Some(app);
    }
    let (name, _publisher) = app_id.split_once('_')?;
    WINDOWS_PACKAGES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|&(_, app)| app)
}

/// What the process `bundle_id` is: a macOS bundle identifier, or a Windows
/// app id, the two never colliding.
pub fn mic_app(bundle_id: &str) -> Option<MicApp> {
    MIC_APPS
        .iter()
        .filter(|(known, _)| same_app(bundle_id, known))
        // The longest match: `com.google.Chrome.beta` before Chrome.
        .max_by_key(|(known, _)| known.len())
        .map(|&(_, app)| app)
        .or_else(|| windows_app(bundle_id))
}

/// A process on the microphone. On Windows the consent store names no
/// process, so `pid` is 0 and `bundle_id` is the app id of `windows_app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicUser {
    pub pid: i32,
    pub bundle_id: String,
}

/// Reads the list `murmur_mic_users` writes: one "pid<TAB>bundle id" line
/// per process.
#[cfg(any(target_os = "macos", test))]
pub fn parse_mic_users(list: &str) -> Vec<MicUser> {
    list.lines()
        .filter_map(|line| {
            let (pid, bundle_id) = line.split_once('\t')?;
            Some(MicUser {
                pid: pid.parse().ok()?,
                bundle_id: bundle_id.to_string(),
            })
        })
        .collect()
}

/// The calls on the microphone now, each once. A browser joins a call on a
/// meeting page, which `page` reads from its windows, and stays in it, in
/// `browser_calls`, while it holds the microphone: windows show only their
/// front tab, and the call goes on behind another. A browser in a call is not
/// read again. Murmur itself is never in a call: its own dictation holds the
/// microphone too.
pub fn calls_on_mic(
    users: &[MicUser],
    own_pid: i32,
    browser_calls: &mut BTreeMap<Browser, CallApp>,
    mut page: impl FnMut(Browser) -> Option<CallApp>,
) -> Vec<CallApp> {
    let mut browsers = Vec::new();
    let mut calls: Vec<CallApp> = users
        .iter()
        .filter(|user| user.pid != own_pid)
        .filter_map(|user| match mic_app(&user.bundle_id)? {
            MicApp::Call(app) => Some(app),
            MicApp::Browser(browser) => {
                if !browsers.contains(&browser) {
                    browsers.push(browser);
                }
                None
            }
        })
        .collect();
    browser_calls.retain(|browser, _| browsers.contains(browser));
    for browser in browsers {
        if browser_calls.contains_key(&browser) {
            continue;
        }
        if let Some(call) = page(browser) {
            browser_calls.insert(browser, call);
        }
    }
    calls.extend(browser_calls.values());
    calls.sort();
    calls.dedup();
    calls
}

/// The meeting a browser shows, from the title of one of its windows or else
/// the address of its front page, which takes longer to read.
pub fn meeting_page(titles: &[String], url: impl FnOnce() -> Option<String>) -> Option<CallApp> {
    titles
        .iter()
        .find_map(|title| meeting_title(title))
        .or_else(|| url().as_deref().and_then(meeting_url))
}

/// Google Meet names its tab after the meeting code once the call is on:
/// "Meet - abc-defg-hij".
pub fn meeting_title(title: &str) -> Option<CallApp> {
    let rest = title.trim_start().strip_prefix("Meet ")?;
    rest.starts_with(['-', '\u{2013}', '\u{2014}'])
        .then_some(CallApp::GoogleMeet)
}

/// A meeting's address on each platform's web app.
pub fn meeting_url(url: &str) -> Option<CallApp> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let host = host.to_ascii_lowercase();
    let first = path
        .strip_prefix('/')
        .unwrap_or("")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let within = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if host == "meet.google.com" && is_meet_code(first) {
        Some(CallApp::GoogleMeet)
    } else if [
        "teams.microsoft.com",
        "teams.live.com",
        "teams.cloud.microsoft",
    ]
    .contains(&host.as_str())
    {
        Some(CallApp::Teams)
    } else if within("zoom.us") && first == "wc" {
        Some(CallApp::Zoom)
    } else if host.ends_with(".webex.com") {
        Some(CallApp::Webex)
    } else if within("discord.com") && first == "channels" {
        Some(CallApp::Discord)
    } else {
        None
    }
}

/// A Meet code: three, four and three lowercase letters.
fn is_meet_code(segment: &str) -> bool {
    let parts: Vec<&str> = segment.split('-').collect();
    parts.len() == 3
        && parts
            .iter()
            .zip([3, 4, 3])
            .all(|(part, len)| part.len() == len && part.bytes().all(|b| b.is_ascii_lowercase()))
}

/// What the watcher saw this second.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub calls: Vec<CallApp>,
    /// Whether a meeting records.
    pub recording: bool,
    pub now: Instant,
}

/// What the call prompt asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    /// "Record this meeting?", for a call while no meeting records.
    Start,
    /// "Call ended.", while the meeting recording it goes on.
    Ended,
}

/// The call prompt's buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CallAnswer {
    Record,
    NotNow,
    StopAndTranscribe,
    KeepRecording,
}

impl CallAnswer {
    /// The prompt this answers. An answer to another prompt than the one
    /// showing does nothing.
    pub fn kind(self) -> PromptKind {
        match self {
            Self::Record | Self::NotNow => PromptKind::Start,
            Self::StopAndTranscribe | Self::KeepRecording => PromptKind::Ended,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Prompt(PromptKind, CallApp),
    /// Hide the prompt showing.
    Hide,
}

/// One app's call.
#[derive(Debug, Clone, Copy)]
struct Call {
    /// Since when it holds the microphone without a break.
    on_since: Instant,
    /// Since when it left the microphone; `None` while it holds it.
    off_since: Option<Instant>,
    /// Asked once, answered, or recorded: no other prompt for this call.
    asked: bool,
}

#[derive(Debug, Default)]
pub struct CallWatch {
    calls: BTreeMap<CallApp, Call>,
    /// The prompt showing and its call. "Record" stays until it is
    /// answered, a meeting records, or that call ends; "left the call" until
    /// it is answered, the meeting stops, or the call comes back.
    showing: Option<(PromptKind, CallApp)>,
    /// The calls that held the microphone `PROMPT_AFTER` while the meeting
    /// recorded, since it started or since "Keep recording".
    in_meeting: BTreeSet<CallApp>,
}

impl CallWatch {
    /// Takes in what the watcher saw and says what to show or hide, one
    /// change at a time.
    pub fn update(&mut self, snapshot: &Snapshot) -> Option<Action> {
        let now = snapshot.now;
        for (app, call) in self.calls.iter_mut() {
            if !snapshot.calls.contains(app) && call.off_since.is_none() {
                call.off_since = Some(now);
            }
        }
        // Back after this long, it is a new call.
        self.calls.retain(|_, call| {
            call.off_since
                .is_none_or(|off| now.duration_since(off) < CALL_OVER_AFTER)
        });
        for &app in &snapshot.calls {
            let showing = self.showing == Some((PromptKind::Start, app));
            let call = self.calls.entry(app).or_insert(Call {
                on_since: now,
                off_since: None,
                asked: showing,
            });
            if call.off_since.take().is_some() {
                call.on_since = now;
            }
            call.asked |= snapshot.recording;
        }
        if snapshot.recording {
            let held = self.calls.iter().filter(|(_, call)| {
                call.off_since.is_none() && now.duration_since(call.on_since) >= PROMPT_AFTER
            });
            self.in_meeting.extend(held.map(|(&app, _)| app));
        } else {
            self.in_meeting.clear();
        }

        if let Some((kind, app)) = self.showing {
            let stays = match kind {
                PromptKind::Start => {
                    let ended = self.calls.get(&app).is_none_or(|call| {
                        call.off_since
                            .is_some_and(|off| now.duration_since(off) >= ENDED_AFTER)
                    });
                    !snapshot.recording && !ended
                }
                PromptKind::Ended => {
                    let back = self
                        .in_meeting
                        .iter()
                        .any(|app| snapshot.calls.contains(app));
                    snapshot.recording && !back
                }
            };
            if stays {
                return None;
            }
            self.showing = None;
            return Some(Action::Hide);
        }
        if snapshot.recording {
            let app = self.call_ended(now)?;
            self.showing = Some((PromptKind::Ended, app));
            return Some(Action::Prompt(PromptKind::Ended, app));
        }
        let (&app, call) = self.calls.iter_mut().find(|(_, call)| {
            !call.asked
                && call.off_since.is_none()
                && now.duration_since(call.on_since) >= PROMPT_AFTER
        })?;
        call.asked = true;
        self.showing = Some((PromptKind::Start, app));
        Some(Action::Prompt(PromptKind::Start, app))
    }

    /// The call that left last, once every call of the meeting has been off
    /// the microphone `ENDED_PROMPT_AFTER`. A call gone from `calls` left
    /// before any still there.
    fn call_ended(&self, now: Instant) -> Option<CallApp> {
        let off_since = |app: &CallApp| self.calls.get(app).and_then(|call| call.off_since);
        let all_left = self.in_meeting.iter().all(|app| {
            off_since(app).is_some_and(|off| now.duration_since(off) >= ENDED_PROMPT_AFTER)
                || !self.calls.contains_key(app)
        });
        if !all_left {
            return None;
        }
        self.in_meeting.iter().copied().max_by_key(off_since)
    }

    /// The prompt `kind` was answered: it hides. Returns its call, or `None`
    /// when no such prompt shows any more. "Keep recording" forgets the
    /// calls that left: only a call back on the microphone asks again.
    pub fn answered(&mut self, kind: PromptKind) -> Option<CallApp> {
        let (showing, app) = self.showing?;
        if showing != kind {
            return None;
        }
        self.showing = None;
        if kind == PromptKind::Ended {
            self.in_meeting.clear();
        }
        Some(app)
    }
}

#[cfg(test)]
mod tests;
