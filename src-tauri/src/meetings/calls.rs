//! Which calls are on, from the apps holding the microphone, and when to
//! offer to record one or to stop. Pure: `call_watch.rs` polls the system
//! and shows what this decides. Nothing here starts or stops a meeting.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long an app holds the microphone before the prompt: a device check
/// or a short voice message is over sooner.
pub const PROMPT_AFTER: Duration = Duration::from_secs(3);
/// Off the microphone this long while Murmur records, the call has ended.
/// Slack and Zoom drop the microphone for a moment on mute or a device
/// change.
pub const ENDED_AFTER: Duration = Duration::from_secs(15);
/// Off this long, the call is over: the app's next use of the microphone is
/// a new call, which "Not now" no longer mutes.
pub const CALL_OVER_AFTER: Duration = Duration::from_secs(30);
/// A prompt nobody answers hides itself after this.
pub const PROMPT_SHOWN_FOR: Duration = Duration::from_secs(20);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Browser {
    Safari,
    Chrome,
    Arc,
    Edge,
    Brave,
    Firefox,
}

impl Browser {
    /// The app whose windows show the page. The process on the microphone is
    /// often one of its helpers, or WebKit's for Safari.
    pub fn bundle_id(self) -> &'static str {
        match self {
            Self::Safari => "com.apple.Safari",
            Self::Chrome => "com.google.Chrome",
            Self::Arc => "company.thebrowser.Browser",
            Self::Edge => "com.microsoft.edgemac",
            Self::Brave => "com.brave.Browser",
            Self::Firefox => "org.mozilla.firefox",
        }
    }

    /// Whether the address of its front page is read as well as the window
    /// titles.
    pub fn reads_url(self) -> bool {
        matches!(self, Self::Safari | Self::Chrome)
    }
}

/// What a process on the microphone is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicApp {
    Call(CallApp),
    /// A call only on a meeting page.
    Browser(Browser),
}

const MIC_APPS: [(&str, MicApp); 21] = [
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
    ("org.mozilla.firefox", MicApp::Browser(Browser::Firefox)),
    // Firefox's pages run in this process.
    (
        "org.mozilla.plugincontainer",
        MicApp::Browser(Browser::Firefox),
    ),
    ("com.google.Chrome.beta", MicApp::Browser(Browser::Chrome)),
    ("com.google.Chrome.canary", MicApp::Browser(Browser::Chrome)),
    ("com.microsoft.edgemac.Beta", MicApp::Browser(Browser::Edge)),
];

/// `bundle_id` is `known` or one of its helpers, such as
/// `com.google.Chrome.helper`.
fn same_app(bundle_id: &str, known: &str) -> bool {
    bundle_id.len() >= known.len()
        && bundle_id.is_char_boundary(known.len())
        && bundle_id[..known.len()].eq_ignore_ascii_case(known)
        && matches!(bundle_id.as_bytes().get(known.len()), None | Some(b'.'))
}

pub fn mic_app(bundle_id: &str) -> Option<MicApp> {
    MIC_APPS
        .iter()
        .filter(|(known, _)| same_app(bundle_id, known))
        // The longest match: `com.google.Chrome.beta` before Chrome.
        .max_by_key(|(known, _)| known.len())
        .map(|&(_, app)| app)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicUser {
    pub pid: i32,
    pub bundle_id: String,
}

/// Reads the list `murmur_mic_users` writes: one "pid<TAB>bundle id" line
/// per process.
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
    pub dictating: bool,
    /// The meeting recording, by when its first part started: it stays the
    /// same through its parts.
    pub meeting: Option<i64>,
    pub now: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// "Record this meeting?"
    Prompt(CallApp),
    /// "The call ended. Stop recording?"
    Ended(CallApp),
    /// Hide the prompt showing.
    Hide,
}

/// Which question the prompt asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    Record,
    Stop,
}

#[derive(Debug, Clone, Copy)]
struct Showing {
    question: Question,
    app: CallApp,
    since: Instant,
}

/// One app's call.
#[derive(Debug, Clone, Copy)]
struct Call {
    /// Since when it holds the microphone without a break.
    on_since: Instant,
    /// Since when it left the microphone; `None` while it holds it.
    off_since: Option<Instant>,
    /// Asked once, answered, or recorded: no other "Record" prompt for this
    /// call.
    asked: bool,
    /// The meeting that recorded while the call held the microphone.
    recorded_in: Option<i64>,
    /// "Not now", or the "Record" prompt let go unanswered: on the
    /// microphone, such as Discord idling in a voice channel, the call holds
    /// back no "Stop recording?".
    passed_over: bool,
}

#[derive(Debug, Default)]
pub struct CallWatch {
    calls: BTreeMap<CallApp, Call>,
    showing: Option<Showing>,
    /// When the call "Stop recording?" last asked about left the microphone:
    /// each leave is asked about once.
    stop_asked: Option<Instant>,
}

impl CallWatch {
    /// Takes in what the watcher saw and says what to show or hide, one
    /// change at a time.
    pub fn update(&mut self, snapshot: &Snapshot) -> Option<Action> {
        let now = snapshot.now;
        for &app in &snapshot.calls {
            let call = self.calls.entry(app).or_insert(Call {
                on_since: now,
                off_since: None,
                asked: false,
                recorded_in: None,
                passed_over: false,
            });
            if let Some(off) = call.off_since.take() {
                call.on_since = now;
                // Kept by the meeting, back after a call's end: a new call.
                if now.duration_since(off) >= CALL_OVER_AFTER {
                    call.passed_over = false;
                }
            }
            if snapshot.meeting.is_some() {
                call.recorded_in = snapshot.meeting;
                call.asked = true;
            }
        }
        for (app, call) in self.calls.iter_mut() {
            if !snapshot.calls.contains(app) && call.off_since.is_none() {
                call.off_since = Some(now);
            }
        }
        // The calls a meeting recorded stay while it records: a dictation
        // may hold "Stop recording?" back past `CALL_OVER_AFTER`.
        self.calls.retain(|_, call| {
            call.off_since
                .is_none_or(|off| now.duration_since(off) < CALL_OVER_AFTER)
                || (snapshot.meeting.is_some() && call.recorded_in == snapshot.meeting)
        });
        // Every call on the microphone while a meeting records is recorded.
        let recording_a_call = snapshot.meeting.is_some()
            && self
                .calls
                .values()
                .any(|call| call.off_since.is_none() && !call.passed_over);

        if let Some(shown) = self.showing {
            let expired = now.duration_since(shown.since) >= PROMPT_SHOWN_FOR;
            let stale = snapshot.dictating
                || expired
                || match shown.question {
                    Question::Record => snapshot.meeting.is_some(),
                    Question::Stop => snapshot.meeting.is_none() || recording_a_call,
                };
            if !stale {
                return None;
            }
            self.showing = None;
            match shown.question {
                Question::Record
                    if expired && !snapshot.dictating && snapshot.meeting.is_none() =>
                {
                    self.pass_over(shown.app)
                }
                // Asked again once the dictation is over.
                Question::Stop if snapshot.dictating => self.stop_asked = None,
                _ => {}
            }
            return Some(Action::Hide);
        }
        if snapshot.dictating {
            return None;
        }
        if snapshot.meeting.is_some() {
            if recording_a_call {
                return None;
            }
            // About the last call the meeting recorded to leave.
            let (off, app) = self
                .calls
                .iter()
                .filter(|(_, call)| call.recorded_in == snapshot.meeting)
                .filter_map(|(&app, call)| call.off_since.map(|off| (off, app)))
                .max()?;
            if self.stop_asked == Some(off) || now.duration_since(off) < ENDED_AFTER {
                return None;
            }
            self.stop_asked = Some(off);
            self.show(Question::Stop, app, now);
            return Some(Action::Ended(app));
        }
        let (&app, call) = self.calls.iter_mut().find(|(_, call)| {
            !call.asked
                && call.off_since.is_none()
                && now.duration_since(call.on_since) >= PROMPT_AFTER
        })?;
        call.asked = true;
        self.show(Question::Record, app, now);
        Some(Action::Prompt(app))
    }

    fn show(&mut self, question: Question, app: CallApp, since: Instant) {
        self.showing = Some(Showing {
            question,
            app,
            since,
        });
    }

    /// The prompt asking `question` was answered, "Record" or "Stop" when
    /// `accept`: it hides. Returns its app, or `None` when that prompt no
    /// longer shows.
    pub fn answered(&mut self, question: Question, accept: bool) -> Option<CallApp> {
        let shown = self.showing.filter(|shown| shown.question == question)?;
        self.showing = None;
        if question == Question::Record && !accept {
            self.pass_over(shown.app);
        }
        Some(shown.app)
    }

    fn pass_over(&mut self, app: CallApp) {
        if let Some(call) = self.calls.get_mut(&app) {
            call.passed_over = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN_PID: i32 = 4242;

    fn user(pid: i32, bundle_id: &str) -> MicUser {
        MicUser {
            pid,
            bundle_id: bundle_id.to_string(),
        }
    }

    /// Drives a watch one second at a time, as the watcher polls.
    struct Clock {
        watch: CallWatch,
        browser_calls: BTreeMap<Browser, CallApp>,
        start: Instant,
        second: u64,
    }

    impl Clock {
        fn new() -> Self {
            Self {
                watch: CallWatch::default(),
                browser_calls: BTreeMap::new(),
                start: Instant::now(),
                second: 0,
            }
        }

        /// `seconds` polls with `users` on the microphone, their browser's
        /// front tab showing `page`.
        fn run_mic(
            &mut self,
            seconds: u64,
            users: &[MicUser],
            page: Option<CallApp>,
            meeting: Option<i64>,
        ) -> Vec<(u64, Action)> {
            let mut actions = Vec::new();
            for _ in 0..seconds {
                let calls = calls_on_mic(users, OWN_PID, &mut self.browser_calls, |_| page);
                actions.extend(self.run(1, &calls, false, meeting));
            }
            actions
        }

        /// `seconds` polls with `calls` on the microphone, the first at
        /// second 0; the actions taken, at the second they were.
        fn run(
            &mut self,
            seconds: u64,
            calls: &[CallApp],
            dictating: bool,
            meeting: Option<i64>,
        ) -> Vec<(u64, Action)> {
            (0..seconds)
                .filter_map(|_| {
                    let at = self.second;
                    self.second += 1;
                    let snapshot = Snapshot {
                        calls: calls.to_vec(),
                        dictating,
                        meeting,
                        now: self.start + Duration::from_secs(at),
                    };
                    self.watch.update(&snapshot).map(|action| (at, action))
                })
                .collect()
        }
    }

    const ZOOM: &[CallApp] = &[CallApp::Zoom];

    #[test]
    fn zoom_on_the_microphone_for_three_seconds_prompts_once() {
        let mut clock = Clock::new();
        assert_eq!(clock.run(3, ZOOM, false, None), []);
        assert_eq!(
            clock.run(1, ZOOM, false, None),
            [(3, Action::Prompt(CallApp::Zoom))]
        );
        assert_eq!(clock.run(10, ZOOM, false, None), []);
        assert_eq!(
            clock.watch.answered(Question::Record, true),
            Some(CallApp::Zoom)
        );
        assert_eq!(clock.run(60, ZOOM, false, None), []);
    }

    #[test]
    fn a_short_use_of_the_microphone_does_not_prompt() {
        let mut clock = Clock::new();
        assert_eq!(clock.run(2, ZOOM, false, None), []);
        assert_eq!(clock.run(5, &[], false, None), []);
    }

    #[test]
    fn a_dictation_running_holds_the_prompt_back() {
        let mut clock = Clock::new();
        assert_eq!(clock.run(10, ZOOM, true, None), []);
    }

    #[test]
    fn a_dictation_that_starts_hides_the_prompt() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        assert_eq!(clock.run(1, ZOOM, true, None), [(4, Action::Hide)]);
        assert_eq!(clock.run(10, ZOOM, false, None), []);
    }

    #[test]
    fn murmur_itself_on_the_microphone_is_never_a_call() {
        let users = [user(OWN_PID, "com.tinyspeck.slackmacgap")];
        assert_eq!(
            calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |_| None),
            []
        );
        let users = [user(OWN_PID, "com.murmur.app"), user(77, "us.zoom.xos")];
        assert_eq!(
            calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |_| None),
            [CallApp::Zoom]
        );
    }

    #[test]
    fn not_now_mutes_the_app_until_it_leaves_the_microphone_for_30_seconds() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        assert_eq!(
            clock.watch.answered(Question::Record, false),
            Some(CallApp::Zoom)
        );
        clock.run(29, &[], false, None);
        assert_eq!(clock.run(20, ZOOM, false, None), []);
        clock.run(31, &[], false, None);
        assert_eq!(
            clock.run(4, ZOOM, false, None),
            [(87, Action::Prompt(CallApp::Zoom))]
        );
    }

    #[test]
    fn an_unanswered_prompt_hides_after_20_seconds_and_does_not_come_back() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        assert_eq!(clock.run(30, ZOOM, false, None), [(23, Action::Hide)]);
    }

    #[test]
    fn a_short_drop_does_not_end_the_call_and_15_seconds_do_while_recording() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        clock.watch.answered(Question::Record, true);
        clock.run(60, ZOOM, false, Some(1));
        assert_eq!(clock.run(5, &[], false, Some(1)), []);
        assert_eq!(clock.run(30, ZOOM, false, Some(1)), []);
        assert_eq!(
            clock.run(16, &[], false, Some(1)),
            [(114, Action::Ended(CallApp::Zoom))]
        );
        assert_eq!(
            clock.watch.answered(Question::Stop, false),
            Some(CallApp::Zoom)
        );
        assert_eq!(clock.run(30, &[], false, Some(1)), []);
    }

    /// One step of an end-prompt script.
    enum Step {
        /// Seconds of polls with these calls on the microphone, and the
        /// meeting recording.
        Poll(u64, &'static [CallApp], Option<i64>),
        /// The same while a dictation runs.
        Dictate(u64, &'static [CallApp], Option<i64>),
        /// The prompt asking this question is answered, "Record" or "Stop"
        /// when true.
        Answer(Question, bool),
    }

    use Step::{Answer, Dictate, Poll};

    const DISCORD: &[CallApp] = &[CallApp::Discord];
    const SLACK: &[CallApp] = &[CallApp::Slack];
    const ZOOM_AND_SLACK: &[CallApp] = &[CallApp::Zoom, CallApp::Slack];
    const ZOOM_AND_DISCORD: &[CallApp] = &[CallApp::Zoom, CallApp::Discord];

    /// A script's name, its steps, and every action they must give.
    type Script = (&'static str, &'static [Step], &'static [(u64, Action)]);

    /// Every way "Stop recording?" has gone wrong in an audit or a review,
    /// and what the rule that fixes them gives besides.
    const END_PROMPT_CORPUS: &[Script] = &[
        (
            "round 48: a dictation through the end of a recorded call delays the prompt",
            &[
                Poll(30, ZOOM, Some(7)),
                Poll(10, &[], Some(7)),
                Dictate(30, &[], Some(7)),
                Poll(1, &[], Some(7)),
            ],
            &[(70, Action::Ended(CallApp::Zoom))],
        ),
        (
            "round 48: a prompt a dictation hid is asked again after it",
            &[
                Poll(30, ZOOM, Some(7)),
                Poll(16, &[], Some(7)),
                Dictate(5, &[], Some(7)),
                Poll(5, &[], Some(7)),
            ],
            &[
                (45, Action::Ended(CallApp::Zoom)),
                (46, Action::Hide),
                (51, Action::Ended(CallApp::Zoom)),
            ],
        ),
        (
            "round 49: a recorded call still on the microphone holds the prompt back",
            &[
                Poll(30, ZOOM_AND_SLACK, Some(7)),
                Poll(60, ZOOM, Some(7)),
                Poll(16, &[], Some(7)),
                Answer(Question::Stop, false),
                Poll(30, &[], Some(7)),
            ],
            &[(105, Action::Ended(CallApp::Zoom))],
        ),
        (
            "round 49: another recorded call coming back hides the prompt",
            &[
                Poll(30, ZOOM_AND_SLACK, Some(7)),
                Poll(5, ZOOM, Some(7)),
                Poll(16, &[], Some(7)),
                Poll(2, SLACK, Some(7)),
            ],
            &[(50, Action::Ended(CallApp::Zoom)), (51, Action::Hide)],
        ),
        (
            "round 50: Discord idling after Not now holds nothing back",
            &[
                Poll(10, DISCORD, None),
                Answer(Question::Record, false),
                Poll(4, ZOOM_AND_DISCORD, None),
                Answer(Question::Record, true),
                Poll(60, ZOOM_AND_DISCORD, Some(7)),
                Poll(16, DISCORD, Some(7)),
            ],
            &[
                (3, Action::Prompt(CallApp::Discord)),
                (13, Action::Prompt(CallApp::Zoom)),
                (89, Action::Ended(CallApp::Zoom)),
            ],
        ),
        (
            "round 50: Discord idling after its prompt expired holds nothing back",
            &[
                Poll(30, DISCORD, None),
                Poll(4, ZOOM_AND_DISCORD, None),
                Answer(Question::Record, true),
                Poll(60, ZOOM_AND_DISCORD, Some(7)),
                Poll(16, DISCORD, Some(7)),
            ],
            &[
                (3, Action::Prompt(CallApp::Discord)),
                (23, Action::Hide),
                (33, Action::Prompt(CallApp::Zoom)),
                (109, Action::Ended(CallApp::Zoom)),
            ],
        ),
        (
            "a call passed over that leaves after the prompt about another asks again",
            &[
                Poll(10, DISCORD, None),
                Answer(Question::Record, false),
                Poll(4, ZOOM_AND_DISCORD, None),
                Answer(Question::Record, true),
                Poll(60, ZOOM_AND_DISCORD, Some(7)),
                Poll(16, DISCORD, Some(7)),
                Answer(Question::Stop, false),
                Poll(16, &[], Some(7)),
            ],
            &[
                (3, Action::Prompt(CallApp::Discord)),
                (13, Action::Prompt(CallApp::Zoom)),
                (89, Action::Ended(CallApp::Zoom)),
                (105, Action::Ended(CallApp::Discord)),
            ],
        ),
        (
            "round 50 review: the call recorded from its prompt outlasts a short one",
            &[
                Poll(4, ZOOM, None),
                Answer(Question::Record, true),
                Poll(60, ZOOM, Some(7)),
                Poll(2, ZOOM_AND_SLACK, Some(7)),
                Poll(60, ZOOM, Some(7)),
                Poll(16, &[], Some(7)),
            ],
            &[
                (3, Action::Prompt(CallApp::Zoom)),
                (141, Action::Ended(CallApp::Zoom)),
            ],
        ),
        (
            "round 50 review: two calls on the microphone before the meeting",
            &[
                Poll(4, ZOOM_AND_SLACK, None),
                Answer(Question::Record, true),
                Poll(30, ZOOM_AND_SLACK, Some(7)),
                Poll(60, ZOOM, Some(7)),
                Poll(16, &[], Some(7)),
            ],
            &[
                (3, Action::Prompt(CallApp::Zoom)),
                (109, Action::Ended(CallApp::Zoom)),
            ],
        ),
    ];

    #[test]
    fn the_end_prompt_corpus() {
        let mut wrong = Vec::new();
        for (name, steps, expected) in END_PROMPT_CORPUS {
            let mut clock = Clock::new();
            let mut actions = Vec::new();
            for step in *steps {
                match *step {
                    Poll(seconds, calls, meeting) => {
                        actions.extend(clock.run(seconds, calls, false, meeting))
                    }
                    Dictate(seconds, calls, meeting) => {
                        actions.extend(clock.run(seconds, calls, true, meeting))
                    }
                    Answer(question, accept) => assert!(
                        clock.watch.answered(question, accept).is_some(),
                        "{name}: no prompt to answer"
                    ),
                }
            }
            if actions != *expected {
                wrong.push(format!("{name}\n  gave {actions:?}\n  want {expected:?}"));
            }
        }
        assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
    }

    #[test]
    fn a_call_that_ends_with_nothing_recording_asks_nothing() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        clock.watch.answered(Question::Record, false);
        assert_eq!(clock.run(60, &[], false, None), []);
    }

    #[test]
    fn a_meeting_started_by_the_shortcut_gets_no_start_prompt_but_an_end_prompt() {
        let mut clock = Clock::new();
        assert_eq!(clock.run(30, ZOOM, false, Some(7)), []);
        assert_eq!(
            clock.run(16, &[], false, Some(7)),
            [(45, Action::Ended(CallApp::Zoom))]
        );
    }

    #[test]
    fn stopping_the_recording_hides_the_end_prompt() {
        let mut clock = Clock::new();
        clock.run(30, ZOOM, false, Some(7));
        clock.run(16, &[], false, Some(7));
        assert_eq!(clock.run(1, &[], false, None), [(46, Action::Hide)]);
    }

    #[test]
    fn the_call_coming_back_hides_the_end_prompt() {
        let mut clock = Clock::new();
        clock.run(30, ZOOM, false, Some(7));
        clock.run(16, &[], false, Some(7));
        assert_eq!(clock.run(1, ZOOM, false, Some(7)), [(46, Action::Hide)]);
    }

    #[test]
    fn a_meeting_started_some_other_way_hides_the_start_prompt() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        assert_eq!(clock.run(1, ZOOM, false, Some(7)), [(4, Action::Hide)]);
    }

    #[test]
    fn a_browser_prompts_only_on_a_meeting_page() {
        let users = [user(500, "com.google.Chrome.helper")];
        assert_eq!(
            calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |_| None),
            []
        );
        let titles = ["Voice memo".to_string()];
        let read = |browser: Browser| {
            assert_eq!(browser, Browser::Chrome);
            meeting_page(&titles, || {
                Some("https://meet.google.com/abc-defg-hij".to_string())
            })
        };
        assert_eq!(
            calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), read),
            [CallApp::GoogleMeet]
        );
        let landing = || Some("https://meet.google.com/landing".to_string());
        assert_eq!(meeting_page(&titles, landing), None);
    }

    const MEET: Option<CallApp> = Some(CallApp::GoogleMeet);

    fn chrome() -> [MicUser; 1] {
        [user(500, "com.google.Chrome.helper")]
    }

    #[test]
    fn a_meet_call_behind_another_tab_goes_on_while_chrome_holds_the_microphone() {
        let mut clock = Clock::new();
        assert_eq!(
            clock.run_mic(4, &chrome(), MEET, None),
            [(3, Action::Prompt(CallApp::GoogleMeet))]
        );
        clock.watch.answered(Question::Record, true);
        assert_eq!(clock.run_mic(10, &chrome(), MEET, Some(1)), []);
        assert_eq!(clock.run_mic(20, &chrome(), None, Some(1)), []);
        assert_eq!(
            clock.run_mic(16, &[], None, Some(1)),
            [(49, Action::Ended(CallApp::GoogleMeet))]
        );
    }

    #[test]
    fn not_now_holds_while_the_meet_call_is_behind_another_tab() {
        let mut clock = Clock::new();
        clock.run_mic(4, &chrome(), MEET, None);
        assert_eq!(
            clock.watch.answered(Question::Record, false),
            Some(CallApp::GoogleMeet)
        );
        assert_eq!(clock.run_mic(40, &chrome(), None, None), []);
        assert_eq!(clock.run_mic(10, &chrome(), MEET, None), []);
    }

    #[test]
    fn a_browser_off_the_microphone_leaves_its_call() {
        let mut clock = Clock::new();
        clock.run_mic(4, &chrome(), MEET, None);
        clock.watch.answered(Question::Record, false);
        clock.run_mic(31, &[], None, None);
        assert_eq!(clock.run_mic(10, &chrome(), None, None), []);
        assert_eq!(
            clock.run_mic(4, &chrome(), MEET, None),
            [(48, Action::Prompt(CallApp::GoogleMeet))]
        );
    }

    #[test]
    fn a_meet_title_spares_reading_the_address() {
        let titles = ["Inbox".to_string(), "Meet - abc-defg-hij".to_string()];
        let url = || -> Option<String> { panic!("the address was read") };
        assert_eq!(meeting_page(&titles, url), Some(CallApp::GoogleMeet));
    }

    #[test]
    fn the_bridge_list_reads_as_processes() {
        assert_eq!(
            parse_mic_users("501\tus.zoom.xos\n77\t\nnot a pid\tx\n"),
            [user(501, "us.zoom.xos"), user(77, "")]
        );
        assert_eq!(parse_mic_users(""), []);
    }

    #[test]
    fn a_browser_with_many_helpers_on_the_microphone_is_read_once() {
        let users = [
            user(500, "com.google.Chrome.helper"),
            user(501, "com.google.Chrome.helper.renderer"),
            user(600, "com.apple.WebKit.GPU"),
        ];
        let mut reads = Vec::new();
        calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |browser| {
            reads.push(browser);
            None
        });
        assert_eq!(reads, [Browser::Chrome, Browser::Safari]);
    }

    #[test]
    fn every_call_app_and_its_helpers_are_known() {
        for (bundle_id, app) in [
            ("us.zoom.xos", CallApp::Zoom),
            ("com.microsoft.teams2", CallApp::Teams),
            ("com.microsoft.teams2.helper", CallApp::Teams),
            ("com.microsoft.teams", CallApp::Teams),
            ("com.tinyspeck.slackmacgap.helper", CallApp::Slack),
            ("com.apple.FaceTime", CallApp::FaceTime),
            ("com.apple.avconferenced", CallApp::FaceTime),
            ("Cisco-Systems.Spark", CallApp::Webex),
            ("com.webex.meetingmanager", CallApp::Webex),
            ("com.hnc.Discord.helper", CallApp::Discord),
            ("US.ZOOM.XOS", CallApp::Zoom),
        ] {
            assert_eq!(mic_app(bundle_id), Some(MicApp::Call(app)), "{bundle_id}");
        }
        for bundle_id in ["us.zoom.xosx", "com.microsoft.teams3", "com.apple", "", "é"] {
            assert_eq!(mic_app(bundle_id), None, "{bundle_id:?}");
        }
        assert_eq!(
            mic_app("com.google.Chrome.beta.helper"),
            Some(MicApp::Browser(Browser::Chrome))
        );
    }

    #[test]
    fn meeting_addresses_of_every_platform_match() {
        for (url, app) in [
            ("https://meet.google.com/abc-defg-hij", CallApp::GoogleMeet),
            (
                "https://meet.google.com/abc-defg-hij?authuser=1",
                CallApp::GoogleMeet,
            ),
            (
                "https://teams.microsoft.com/v2/?meetingjoin=true",
                CallApp::Teams,
            ),
            ("https://teams.live.com/meet/9876543210", CallApp::Teams),
            ("https://teams.cloud.microsoft/", CallApp::Teams),
            ("https://app.zoom.us/wc/123456789/start", CallApp::Zoom),
            ("https://us02web.zoom.us/wc/join/123456789", CallApp::Zoom),
            ("https://acme.webex.com/meet/someone", CallApp::Webex),
            ("https://discord.com/channels/1234/5678", CallApp::Discord),
            ("HTTPS://Meet.Google.com/abc-defg-hij", CallApp::GoogleMeet),
        ] {
            assert_eq!(meeting_url(url), Some(app), "{url}");
        }
    }

    #[test]
    fn pages_that_are_no_meeting_do_not_match() {
        for url in [
            "https://meet.google.com/",
            "https://meet.google.com/landing",
            "https://meet.google.com/abc-defg-hijk",
            "https://meet.google.com.example.org/abc-defg-hij",
            "https://example.org/meet.google.com/abc-defg-hij",
            "https://zoom.us/pricing",
            "https://app.zoom.us/wcx",
            "https://webex.com/",
            "https://notwebex.com/",
            "https://discord.com/app",
            "file:///tmp/teams.microsoft.com",
            "teams.microsoft.com",
            "",
        ] {
            assert_eq!(meeting_url(url), None, "{url}");
        }
    }

    #[test]
    fn a_meet_tab_title_matches_with_any_dash() {
        for title in [
            "Meet - abc-defg-hij",
            "Meet \u{2013} abc-defg-hij",
            "Meet \u{2014} abc-defg-hij - Google Chrome",
        ] {
            assert_eq!(meeting_title(title), Some(CallApp::GoogleMeet), "{title}");
        }
        for title in [
            "Meet",
            "Meeting notes - Docs",
            "Google Meet",
            "Meet-up plans",
        ] {
            assert_eq!(meeting_title(title), None, "{title}");
        }
    }
}
