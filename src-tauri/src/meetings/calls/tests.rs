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
        recording: bool,
    ) -> Vec<(u64, Action)> {
        let mut actions = Vec::new();
        for _ in 0..seconds {
            let calls = calls_on_mic(users, OWN_PID, &mut self.browser_calls, |_| page);
            actions.extend(self.run(1, &calls, recording));
        }
        actions
    }

    /// `seconds` polls with `calls` on the microphone, the first at
    /// second 0; the actions taken, at the second they were.
    fn run(&mut self, seconds: u64, calls: &[CallApp], recording: bool) -> Vec<(u64, Action)> {
        (0..seconds)
            .filter_map(|_| {
                let at = self.second;
                self.second += 1;
                let snapshot = Snapshot {
                    calls: calls.to_vec(),
                    recording,
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
    assert_eq!(clock.run(3, ZOOM, false), []);
    assert_eq!(
        clock.run(1, ZOOM, false),
        [(3, Action::Prompt(PromptKind::Start, CallApp::Zoom))]
    );
    assert_eq!(clock.run(10, ZOOM, false), []);
    assert_eq!(clock.watch.answered(PromptKind::Start), Some(CallApp::Zoom));
    assert_eq!(clock.run(60, ZOOM, false), []);
}

#[test]
fn a_short_use_of_the_microphone_does_not_prompt() {
    let mut clock = Clock::new();
    assert_eq!(clock.run(2, ZOOM, false), []);
    assert_eq!(clock.run(5, &[], false), []);
}

#[test]
fn the_prompt_stays_until_it_is_answered() {
    let mut clock = Clock::new();
    assert_eq!(
        clock.run(600, ZOOM, false),
        [(3, Action::Prompt(PromptKind::Start, CallApp::Zoom))]
    );
}

#[test]
fn the_call_leaving_the_microphone_closes_its_prompt() {
    let mut clock = Clock::new();
    clock.run(4, ZOOM, false);
    assert_eq!(clock.run(14, &[], false), []);
    assert_eq!(clock.run(10, &[], false), [(19, Action::Hide)]);
}

#[test]
fn a_short_drop_of_the_call_keeps_its_prompt() {
    let mut clock = Clock::new();
    clock.run(4, ZOOM, false);
    assert_eq!(clock.run(10, &[], false), []);
    assert_eq!(clock.run(60, ZOOM, false), []);
}

#[test]
fn a_call_back_after_the_mac_slept_through_its_prompt_is_not_asked_again() {
    let mut clock = Clock::new();
    clock.run(4, ZOOM, false);
    clock.run(1, &[], false);
    clock.second += 60;
    assert_eq!(clock.run(5, ZOOM, false), []);
    assert_eq!(clock.watch.answered(PromptKind::Start), Some(CallApp::Zoom));
    assert_eq!(clock.run(20, ZOOM, false), []);
}

const ENDED_ZOOM: Action = Action::Prompt(PromptKind::Ended, CallApp::Zoom);

#[test]
fn ended_prompt_after_call_leaves_mic_while_recording() {
    let mut clock = Clock::new();
    assert_eq!(clock.run(30, ZOOM, true), []);
    assert_eq!(clock.run(3, &[], true), []);
    assert_eq!(clock.run(1, &[], true), [(33, ENDED_ZOOM)]);
    assert_eq!(clock.run(600, &[], true), [], "it never goes on its own");
    assert_eq!(clock.watch.answered(PromptKind::Ended), Some(CallApp::Zoom));
    assert_eq!(clock.run(5, &[], false), []);

    // Recorded from the "Record" prompt.
    let mut clock = Clock::new();
    clock.run(4, ZOOM, false);
    assert_eq!(clock.watch.answered(PromptKind::Start), Some(CallApp::Zoom));
    assert_eq!(clock.run(60, ZOOM, true), []);
    assert_eq!(clock.run(4, &[], true), [(67, ENDED_ZOOM)]);
}

#[test]
fn ended_prompt_hides_when_the_meeting_stops() {
    let mut clock = Clock::new();
    clock.run(10, ZOOM, true);
    assert_eq!(clock.run(4, &[], true), [(13, ENDED_ZOOM)]);
    assert_eq!(clock.run(1, &[], false), [(14, Action::Hide)]);
    assert_eq!(clock.watch.answered(PromptKind::Ended), None);
    assert_eq!(clock.run(60, &[], false), []);
}

#[test]
fn no_ended_prompt_for_manual_meeting_without_call() {
    let mut clock = Clock::new();
    assert_eq!(clock.run(600, &[], true), []);
    clock.run(2, ZOOM, true);
    assert_eq!(clock.run(60, &[], true), [], "shorter than a call");
}

#[test]
fn ended_prompt_hides_when_app_returns() {
    let mut clock = Clock::new();
    clock.run(10, ZOOM, true);
    assert_eq!(clock.run(4, &[], true), [(13, ENDED_ZOOM)]);
    assert_eq!(clock.run(1, ZOOM, true), [(14, Action::Hide)]);
    assert_eq!(clock.run(30, ZOOM, true), []);
    assert_eq!(clock.run(4, &[], true), [(48, ENDED_ZOOM)]);
}

#[test]
fn keep_recording_is_not_asked_again() {
    let mut clock = Clock::new();
    clock.run(10, ZOOM, true);
    clock.run(4, &[], true);
    assert_eq!(clock.watch.answered(PromptKind::Ended), Some(CallApp::Zoom));
    assert_eq!(clock.run(600, &[], true), []);
    clock.run(1, ZOOM, true);
    assert_eq!(clock.run(10, &[], true), [], "a blip is no call");

    clock.run(5, ZOOM, true);
    assert_eq!(clock.run(4, &[], true), [(633, ENDED_ZOOM)]);
}

#[test]
fn an_answer_to_another_prompt_does_nothing() {
    let mut clock = Clock::new();
    clock.run(10, ZOOM, true);
    clock.run(4, &[], true);
    assert_eq!(clock.watch.answered(PromptKind::Start), None);
    assert_eq!(clock.watch.answered(PromptKind::Ended), Some(CallApp::Zoom));
}

#[test]
fn ended_prompt_waits_for_all_call_apps() {
    let mut clock = Clock::new();
    clock.run(10, &[CallApp::Zoom, CallApp::Slack], true);
    assert_eq!(clock.run(60, &[CallApp::Slack], true), []);
    assert_eq!(
        clock.run(4, &[], true),
        [(73, Action::Prompt(PromptKind::Ended, CallApp::Slack))]
    );
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
    clock.run(4, ZOOM, false);
    assert_eq!(clock.watch.answered(PromptKind::Start), Some(CallApp::Zoom));
    clock.run(29, &[], false);
    assert_eq!(clock.run(20, ZOOM, false), []);
    clock.run(31, &[], false);
    assert_eq!(
        clock.run(4, ZOOM, false),
        [(87, Action::Prompt(PromptKind::Start, CallApp::Zoom))]
    );
}

#[test]
fn a_meeting_started_some_other_way_hides_the_prompt() {
    let mut clock = Clock::new();
    clock.run(4, ZOOM, false);
    assert_eq!(clock.run(1, ZOOM, true), [(4, Action::Hide)]);
    assert_eq!(clock.watch.answered(PromptKind::Start), None);
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
fn a_meet_call_behind_another_tab_keeps_its_prompt_while_chrome_holds_the_microphone() {
    let mut clock = Clock::new();
    assert_eq!(
        clock.run_mic(4, &chrome(), MEET, false),
        [(3, Action::Prompt(PromptKind::Start, CallApp::GoogleMeet))]
    );
    assert_eq!(clock.run_mic(30, &chrome(), None, false), []);
    assert_eq!(clock.run_mic(16, &[], None, false), [(49, Action::Hide)]);
}

#[test]
fn not_now_holds_while_the_meet_call_is_behind_another_tab() {
    let mut clock = Clock::new();
    clock.run_mic(4, &chrome(), MEET, false);
    assert_eq!(
        clock.watch.answered(PromptKind::Start),
        Some(CallApp::GoogleMeet)
    );
    assert_eq!(clock.run_mic(40, &chrome(), None, false), []);
    assert_eq!(clock.run_mic(10, &chrome(), MEET, false), []);
}

#[test]
fn a_browser_off_the_microphone_leaves_its_call() {
    let mut clock = Clock::new();
    clock.run_mic(4, &chrome(), MEET, false);
    clock.watch.answered(PromptKind::Start);
    clock.run_mic(31, &[], None, false);
    assert_eq!(clock.run_mic(10, &chrome(), None, false), []);
    assert_eq!(
        clock.run_mic(4, &chrome(), MEET, false),
        [(48, Action::Prompt(PromptKind::Start, CallApp::GoogleMeet))]
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
fn browser_editions_read_their_own_windows_with_or_without_the_stable_app() {
    for (bundle_id, stable_id, reads_url) in [
        ("com.google.Chrome.beta", "com.google.Chrome", true),
        ("com.google.Chrome.canary", "com.google.Chrome", true),
        ("com.microsoft.edgemac.Beta", "com.microsoft.edgemac", false),
    ] {
        for stable_running in [false, true] {
            let mut windows = Vec::new();
            if stable_running {
                windows.push((stable_id, "Voice memo"));
            }
            windows.push((bundle_id, "Meet - abc-defg-hij"));
            let users = [user(500, &format!("{bundle_id}.helper"))];
            let calls = calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |browser| {
                assert_eq!(browser.reads_url(), reads_url);
                // Same exact app-selection predicate as call_watch::browser_call.
                windows.iter().find_map(|(running_id, title)| {
                    (*running_id == browser.bundle_id())
                        .then(|| meeting_title(title))
                        .flatten()
                })
            });
            assert_eq!(calls, [CallApp::GoogleMeet], "{bundle_id}");

            // A meeting window in stable must not turn this edition's
            // microphone use into a call.
            let calls = calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |browser| {
                (browser.bundle_id() == stable_id)
                    .then(|| meeting_title("Meet - abc-defg-hij"))
                    .flatten()
            });
            assert_eq!(calls, [], "{bundle_id}");
        }
    }
}

#[test]
fn simultaneous_browser_editions_keep_separate_cached_calls() {
    let editions = [
        ("com.google.Chrome", CallApp::GoogleMeet),
        ("com.google.Chrome.beta", CallApp::Teams),
        ("com.google.Chrome.canary", CallApp::Zoom),
        ("com.microsoft.edgemac", CallApp::Webex),
        ("com.microsoft.edgemac.Beta", CallApp::Discord),
        ("net.imput.helium", CallApp::Slack),
    ];
    let users: Vec<_> = editions
        .iter()
        .map(|(bundle_id, _)| user(500, &format!("{bundle_id}.helper")))
        .collect();
    let mut browser_calls = BTreeMap::new();
    let mut reads = Vec::new();
    let calls = calls_on_mic(&users, OWN_PID, &mut browser_calls, |browser| {
        reads.push(browser.bundle_id());
        editions
            .iter()
            .find_map(|(bundle_id, call)| (*bundle_id == browser.bundle_id()).then_some(*call))
    });
    assert_eq!(reads, editions.map(|(bundle_id, _)| bundle_id));
    assert_eq!(calls.len(), editions.len());
    assert_eq!(browser_calls.len(), editions.len());

    // Beta leaving the microphone must not borrow stable's cached call,
    // nor keep Beta's cached call alive while only stable remains.
    assert_eq!(
        calls_on_mic(&users[..1], OWN_PID, &mut browser_calls, |_| {
            panic!("the stable browser's cached call was read again")
        }),
        [CallApp::GoogleMeet]
    );
    assert_eq!(browser_calls.len(), 1);
    assert_eq!(
        calls_on_mic(&users[1..2], OWN_PID, &mut browser_calls, |_| None),
        []
    );
    assert!(browser_calls.is_empty());
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
        Some(MicApp::Browser(Browser::ChromeBeta))
    );
    for (bundle_id, browser) in [
        ("net.imput.helium.helper", Browser::Helium),
        ("com.vivaldi.Vivaldi.helper", Browser::Vivaldi),
        ("com.operasoftware.Opera.helper", Browser::Opera),
        ("org.chromium.Chromium.helper", Browser::Chromium),
        ("company.thebrowser.dia.helper", Browser::Dia),
        ("ai.perplexity.comet.helper", Browser::Comet),
        // Dia's id must not fall back to Arc's, nor Arc's to Dia's.
        ("company.thebrowser.Browser.helper", Browser::Arc),
    ] {
        assert_eq!(
            mic_app(bundle_id),
            Some(MicApp::Browser(browser)),
            "{bundle_id}"
        );
    }
}

const TITLES_ONLY_CHROMIUM_BROWSERS: [&str; 6] = [
    "net.imput.helium",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
    "org.chromium.Chromium",
    "company.thebrowser.dia",
    "ai.perplexity.comet",
];

#[test]
fn a_meet_call_in_another_chromium_browser_prompts_from_its_window_titles() {
    for bundle_id in TITLES_ONLY_CHROMIUM_BROWSERS {
        // The browser takes the microphone in its helper, with Chrome open
        // too.
        let users = [user(500, &format!("{bundle_id}.helper"))];
        let windows = [
            ("com.google.Chrome", "Voice memo"),
            (bundle_id, "Meet - abc-defg-hij"),
        ];
        let calls = calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |browser| {
            assert!(!browser.reads_url(), "{bundle_id}: titles only, as Arc");
            // Same exact app-selection predicate as call_watch::browser_call.
            windows.iter().find_map(|(running_id, title)| {
                (*running_id == browser.bundle_id())
                    .then(|| meeting_title(title))
                    .flatten()
            })
        });
        assert_eq!(calls, [CallApp::GoogleMeet], "{bundle_id}");

        // A meeting window in Chrome must not turn this browser's
        // microphone use into a call.
        let calls = calls_on_mic(&users, OWN_PID, &mut BTreeMap::new(), |browser| {
            (browser.bundle_id() == "com.google.Chrome")
                .then(|| meeting_title("Meet - abc-defg-hij"))
                .flatten()
        });
        assert_eq!(calls, [], "{bundle_id}");
    }
}

#[test]
fn chromium_browsers_on_the_mic_together_keep_separate_cached_calls() {
    let users: Vec<_> = TITLES_ONLY_CHROMIUM_BROWSERS
        .iter()
        .map(|bundle_id| user(500, &format!("{bundle_id}.helper")))
        .collect();
    let mut browser_calls = BTreeMap::new();
    let mut reads = Vec::new();
    let calls = calls_on_mic(&users, OWN_PID, &mut browser_calls, |browser| {
        reads.push(browser.bundle_id());
        Some(CallApp::GoogleMeet)
    });
    assert_eq!(reads, TITLES_ONLY_CHROMIUM_BROWSERS);
    assert_eq!(calls, [CallApp::GoogleMeet]);
    assert_eq!(browser_calls.len(), TITLES_ONLY_CHROMIUM_BROWSERS.len());

    // Only the first one left on the microphone keeps its call.
    assert_eq!(
        calls_on_mic(&users[..1], OWN_PID, &mut browser_calls, |_| {
            panic!("a cached call was read again")
        }),
        [CallApp::GoogleMeet]
    );
    assert_eq!(browser_calls.len(), 1);
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
