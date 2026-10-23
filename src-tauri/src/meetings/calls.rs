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
            Self::Teams => "Microsoft Teams",
            Self::Slack => "Slack",
            Self::FaceTime => "FaceTime",
            Self::Webex => "Webex",
            Self::Discord => "Discord",
        }
    }
}

const CALL_APPS: [(&str, CallApp); 10] = [
    ("us.zoom.xos", CallApp::Zoom),
    ("com.microsoft.teams2", CallApp::Teams),
    ("com.microsoft.teams", CallApp::Teams),
    ("com.tinyspeck.slackmacgap", CallApp::Slack),
    ("com.apple.FaceTime", CallApp::FaceTime),
    // FaceTime's calls run in this daemon, not in the app.
    ("com.apple.avconferenced", CallApp::FaceTime),
    ("Cisco-Systems.Spark", CallApp::Webex),
    ("com.webex.meetingmanager", CallApp::Webex),
    ("com.cisco.webexmeetingsapp", CallApp::Webex),
    ("com.hnc.Discord", CallApp::Discord),
];

/// `bundle_id` is `known` or one of its helpers, such as
/// `com.hnc.Discord.helper`.
fn same_app(bundle_id: &str, known: &str) -> bool {
    bundle_id.len() >= known.len()
        && bundle_id.is_char_boundary(known.len())
        && bundle_id[..known.len()].eq_ignore_ascii_case(known)
        && matches!(bundle_id.as_bytes().get(known.len()), None | Some(b'.'))
}

pub fn call_app(bundle_id: &str) -> Option<CallApp> {
    CALL_APPS
        .iter()
        .find(|(known, _)| same_app(bundle_id, known))
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

/// The calls on the microphone now, each once. Murmur itself is never in a
/// call: its own dictation holds the microphone too.
pub fn calls_on_mic(users: &[MicUser], own_pid: i32) -> Vec<CallApp> {
    let mut calls: Vec<CallApp> = users
        .iter()
        .filter(|user| user.pid != own_pid)
        .filter_map(|user| call_app(&user.bundle_id))
        .collect();
    calls.sort();
    calls.dedup();
    calls
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
    /// "Stop recording?" was asked since it left the microphone.
    end_asked: bool,
}

#[derive(Debug, Default)]
pub struct CallWatch {
    calls: BTreeMap<CallApp, Call>,
    showing: Option<Showing>,
}

impl CallWatch {
    /// Takes in what the watcher saw and says what to show or hide, one
    /// change at a time.
    pub fn update(&mut self, snapshot: &Snapshot) -> Option<Action> {
        let now = snapshot.now;
        let mut back = false;
        for &app in &snapshot.calls {
            let call = self.calls.entry(app).or_insert(Call {
                on_since: now,
                off_since: None,
                asked: false,
                recorded_in: None,
                end_asked: false,
            });
            if call.off_since.take().is_some() {
                call.on_since = now;
                call.end_asked = false;
                back |= self
                    .showing
                    .is_some_and(|shown| shown.question == Question::Stop && shown.app == app);
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
        self.calls.retain(|_, call| {
            call.off_since
                .is_none_or(|off| now.duration_since(off) < CALL_OVER_AFTER)
        });

        if let Some(shown) = self.showing {
            let stale = snapshot.dictating
                || back
                || (shown.question == Question::Record && snapshot.meeting.is_some())
                || (shown.question == Question::Stop && snapshot.meeting.is_none())
                || now.duration_since(shown.since) >= PROMPT_SHOWN_FOR;
            if stale {
                self.showing = None;
                return Some(Action::Hide);
            }
            return None;
        }
        if snapshot.dictating {
            return None;
        }
        if let Some(meeting) = snapshot.meeting {
            let (&app, call) = self.calls.iter_mut().find(|(_, call)| {
                call.recorded_in == Some(meeting)
                    && !call.end_asked
                    && call
                        .off_since
                        .is_some_and(|off| now.duration_since(off) >= ENDED_AFTER)
            })?;
            call.end_asked = true;
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

    /// The prompt asking `question` was answered: it hides. Returns its app,
    /// or `None` when that prompt no longer shows.
    pub fn answered(&mut self, question: Question) -> Option<CallApp> {
        let shown = self.showing.filter(|shown| shown.question == question)?;
        self.showing = None;
        Some(shown.app)
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
        start: Instant,
        second: u64,
    }

    impl Clock {
        fn new() -> Self {
            Self {
                watch: CallWatch::default(),
                start: Instant::now(),
                second: 0,
            }
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
        assert_eq!(clock.watch.answered(Question::Record), Some(CallApp::Zoom));
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
        assert_eq!(calls_on_mic(&users, OWN_PID), []);
        let users = [user(OWN_PID, "com.murmur.app"), user(77, "us.zoom.xos")];
        assert_eq!(calls_on_mic(&users, OWN_PID), [CallApp::Zoom]);
    }

    #[test]
    fn not_now_mutes_the_app_until_it_leaves_the_microphone_for_30_seconds() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        assert_eq!(clock.watch.answered(Question::Record), Some(CallApp::Zoom));
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
        clock.watch.answered(Question::Record);
        clock.run(60, ZOOM, false, Some(1));
        assert_eq!(clock.run(5, &[], false, Some(1)), []);
        assert_eq!(clock.run(30, ZOOM, false, Some(1)), []);
        assert_eq!(
            clock.run(16, &[], false, Some(1)),
            [(114, Action::Ended(CallApp::Zoom))]
        );
        assert_eq!(clock.watch.answered(Question::Stop), Some(CallApp::Zoom));
        assert_eq!(clock.run(30, &[], false, Some(1)), []);
    }

    #[test]
    fn a_call_that_ends_with_nothing_recording_asks_nothing() {
        let mut clock = Clock::new();
        clock.run(4, ZOOM, false, None);
        clock.watch.answered(Question::Record);
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
    fn the_bridge_list_reads_as_processes() {
        assert_eq!(
            parse_mic_users("501\tus.zoom.xos\n77\t\nnot a pid\tx\n"),
            [user(501, "us.zoom.xos"), user(77, "")]
        );
        assert_eq!(parse_mic_users(""), []);
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
            assert_eq!(call_app(bundle_id), Some(app), "{bundle_id}");
        }
        for bundle_id in ["us.zoom.xosx", "com.microsoft.teams3", "com.apple", "", "é"] {
            assert_eq!(call_app(bundle_id), None, "{bundle_id:?}");
        }
    }
}
