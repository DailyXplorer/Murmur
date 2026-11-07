use super::*;
use crate::meetings::store::MeetingStatus;
use crate::meetings::trash::tests::{notes, FolderTrash};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, entry: impl Into<String>) {
        self.0.lock().unwrap().push(entry.into());
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

struct FakeTrack {
    name: &'static str,
    log: Log,
    failed: Arc<AtomicBool>,
    interrupted: Arc<AtomicBool>,
    played: bool,
    dropped_ms: u64,
}

impl Track for FakeTrack {
    fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
    fn interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }
    fn dropped_ms(&self) -> u64 {
        self.dropped_ms
    }
    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        self.log
            .push(format!("finish {} {}s", self.name, duration.as_secs()));
        Ok(self.played)
    }
}

struct FakeBackend {
    support: SystemAudioSupport,
    mic_works: bool,
    system_works: bool,
    log: Log,
    failed: Arc<AtomicBool>,
    mic_interrupted: Arc<AtomicBool>,
    system_interrupted: Arc<AtomicBool>,
    free_bytes: Option<u64>,
    /// Another app plays sound while system audio records.
    system_played: bool,
    microphone_denied: bool,
    system_audio_denied: bool,
    /// Audio each track dropped.
    dropped_ms: u64,
    /// When the Mac is kept awake.
    awake: Log,
    /// Where the Mac plays sound.
    route: OutputRoute,
}

/// Holds the Mac awake until dropped.
struct FakeAwake(Log);

impl Drop for FakeAwake {
    fn drop(&mut self) {
        self.0.push("awake off");
    }
}

impl FakeBackend {
    fn track(&self, name: &'static str, works: bool) -> Result<Box<dyn Track>, String> {
        if !works {
            return Err(format!("{name} unavailable"));
        }
        self.log.push(format!("start {name}"));
        Ok(Box::new(FakeTrack {
            name,
            log: self.log.clone(),
            failed: Arc::clone(&self.failed),
            interrupted: Arc::clone(if name == "mic" {
                &self.mic_interrupted
            } else {
                &self.system_interrupted
            }),
            played: name == "system" && self.system_played,
            dropped_ms: self.dropped_ms,
        }))
    }
}

impl CaptureBackend for FakeBackend {
    fn system_audio(&self) -> SystemAudioSupport {
        self.support
    }
    fn start_mic(&self, dir: &Path, _: Instant) -> Result<Box<dyn Track>, String> {
        assert!(dir.is_dir());
        self.track("mic", self.mic_works)
    }
    fn start_system(&self, dir: &Path, _: Instant) -> Result<Box<dyn Track>, String> {
        assert!(dir.is_dir());
        self.track("system", self.system_works)
    }
    fn free_bytes(&self, dir: &Path) -> Option<u64> {
        assert!(dir.is_dir());
        self.free_bytes
    }
    fn microphone_denied(&self) -> bool {
        self.microphone_denied
    }
    fn system_audio_denied(&self) -> bool {
        self.system_audio_denied
    }
    fn keep_awake(&self) -> Option<Box<dyn Send>> {
        self.awake.push("awake on");
        Some(Box::new(FakeAwake(self.awake.clone())))
    }
    fn output_route(&self) -> OutputRoute {
        self.route
    }
}

struct FakeNotifier(Log);

impl Notifier for FakeNotifier {
    fn runtime_changed(&self, runtime: &MeetingRuntime) {
        self.0.push(match runtime {
            MeetingRuntime::Idle => "idle".to_string(),
            MeetingRuntime::Recording { tracks, .. } => format!("recording {tracks:?}"),
        });
    }
    fn alert(&self, alert: MeetingAlert) {
        self.0.push(format!("alert {alert:?}"));
    }
    fn recorded(&self, _: i64) {
        self.0.push("recorded");
    }
}

#[derive(Clone)]
struct FakeClock {
    origin: Instant,
    elapsed: Arc<Mutex<Duration>>,
    /// Wall-clock time without `Instant` time, as during a Mac's sleep.
    asleep: Arc<Mutex<Duration>>,
}

impl FakeClock {
    fn advance(&self, by: Duration) {
        *self.elapsed.lock().unwrap() += by;
    }

    fn sleep(&self, by: Duration) {
        *self.asleep.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.origin + *self.elapsed.lock().unwrap()
    }
    fn unix_ms(&self) -> i64 {
        let wall = *self.elapsed.lock().unwrap() + *self.asleep.lock().unwrap();
        1_760_000_000_000 + wall.as_millis() as i64
    }
}

struct Harness {
    coordinator: Coordinator<FakeBackend, FakeNotifier, FakeClock>,
    store: Arc<MeetingsStore>,
    clock: FakeClock,
    log: Log,
    failed: Arc<AtomicBool>,
    mic_interrupted: Arc<AtomicBool>,
    system_interrupted: Arc<AtomicBool>,
    awake: Log,
    root: PathBuf,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// Where a test's Trash is: next to the meetings folder, never the
    /// real one.
    fn trash_folder(&self) -> PathBuf {
        self._dir.path().join("trash")
    }
}

fn harness(support: SystemAudioSupport, mic_works: bool, system_works: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = MeetingsStore::open(dir.path().join("meetings.db"), chrono::Utc::now());
    let store = Arc::new(store);
    let root = dir.path().join("meetings");
    std::fs::create_dir_all(&root).unwrap();
    let log = Log::default();
    let failed = Arc::new(AtomicBool::new(false));
    let mic_interrupted = Arc::new(AtomicBool::new(false));
    let system_interrupted = Arc::new(AtomicBool::new(false));
    let awake = Log::default();
    let clock = FakeClock {
        origin: Instant::now(),
        elapsed: Arc::default(),
        asleep: Arc::default(),
    };
    let backend = FakeBackend {
        support,
        mic_works,
        system_works,
        log: log.clone(),
        failed: Arc::clone(&failed),
        mic_interrupted: Arc::clone(&mic_interrupted),
        system_interrupted: Arc::clone(&system_interrupted),
        free_bytes: Some(100_000_000_000),
        system_played: false,
        microphone_denied: false,
        system_audio_denied: false,
        dropped_ms: 0,
        awake: awake.clone(),
        route: OutputRoute::Speakers,
    };
    Harness {
        coordinator: Coordinator::new(
            backend,
            FakeNotifier(log.clone()),
            clock.clone(),
            Arc::clone(&store),
            root.clone(),
        ),
        store,
        clock,
        log,
        failed,
        mic_interrupted,
        system_interrupted,
        awake,
        root,
        _dir: dir,
    }
}

fn working() -> Harness {
    harness(SystemAudioSupport::Available, true, true)
}

fn statuses(store: &MeetingsStore) -> Vec<(MeetingStatus, MeetingTracks, i64)> {
    store
        .list(None, 10)
        .unwrap()
        .meetings
        .into_iter()
        .map(|meeting| (meeting.status, meeting.tracks, meeting.duration_ms))
        .collect()
}

#[test]
fn start_then_stop_records_both_tracks_for_processing() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert!(matches!(
        h.coordinator.runtime(),
        MeetingRuntime::Recording { id: running, .. } if running == id
    ));
    h.clock.advance(Duration::from_secs(90));

    assert_eq!(h.coordinator.stop().unwrap(), id);

    assert_eq!(
        h.log.take(),
        [
            "start mic",
            "start system",
            "recording Both",
            "finish mic 90s",
            "finish system 90s",
            "idle",
            "recorded"
        ]
    );
    assert_eq!(h.coordinator.runtime(), MeetingRuntime::Idle);
    assert_eq!(
        statuses(&h.store),
        [(MeetingStatus::Processing, MeetingTracks::Both, 90_000)]
    );
    assert!(paths::meeting_dir(&h.root, id).is_dir());
}

/// A full disk refuses the database write that ends the recording too:
/// the meeting is saved for processing once there is room, not left on
/// "Recording" until the next launch. Unix file modes stand in for the
/// full disk.
#[cfg(unix)]
#[test]
fn a_close_the_disk_refused_is_saved_once_it_has_room() {
    use std::os::unix::fs::PermissionsExt;
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.clock.advance(Duration::from_secs(90));
    let db = h._dir.path().join("meetings.db");
    let writable = |yes: bool| {
        let mode = if yes { 0o644 } else { 0o444 };
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(mode)).unwrap();
    };

    writable(false);
    assert!(h.coordinator.stop().is_err());
    assert!(h.coordinator.tick(id), "the close is tried again");
    writable(true);
    h.log.take();
    assert!(!h.coordinator.tick(id), "the close is done");

    assert_eq!(h.log.take(), ["recorded"]);
    assert_eq!(
        statuses(&h.store),
        [(MeetingStatus::Processing, MeetingTracks::Both, 90_000)]
    );
}

#[test]
fn a_meeting_remembers_whether_another_app_played_sound() {
    let mut h = working();
    let quiet = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    h.coordinator.backend.system_played = true;
    let played = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();

    assert!(!h.store.system_played(quiet).unwrap());
    assert!(h.store.system_played(played).unwrap());
}

#[test]
fn a_meeting_keeps_how_much_audio_its_tracks_dropped() {
    let mut h = working();
    let clean = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    h.coordinator.backend.dropped_ms = 400;
    let dropped = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();

    assert_eq!(h.store.dropped_ms(clean).unwrap(), 0);
    assert_eq!(h.store.dropped_ms(dropped).unwrap(), 800, "both tracks");
}

#[test]
fn a_nearly_full_disk_warns_and_still_records() {
    let mut h = working();
    h.coordinator.backend.free_bytes = Some(LOW_DISK_BYTES - 1);
    h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(
        h.log.take(),
        [
            "start mic",
            "start system",
            "recording Both",
            "alert LowDiskSpace"
        ]
    );
    h.coordinator.stop().unwrap();

    // Enough space, or a size that cannot be read: no warning.
    for free_bytes in [Some(LOW_DISK_BYTES), None] {
        h.log.take();
        h.coordinator.backend.free_bytes = free_bytes;
        h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        assert!(!h.log.take().iter().any(|entry| entry.starts_with("alert")));
        h.coordinator.stop().unwrap();
    }
}

#[test]
fn a_denied_microphone_warns_at_start_and_the_meeting_records() {
    let mut h = working();
    h.coordinator.backend.microphone_denied = true;
    h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(
        h.log.take(),
        [
            "start mic",
            "start system",
            "alert MicrophoneDenied",
            "recording Both"
        ]
    );
    h.coordinator.stop().unwrap();

    // System audio alone does not use the microphone.
    h.log.take();
    h.coordinator.start(MeetingCaptureMode::SystemOnly).unwrap();
    assert!(!h.log.take().iter().any(|entry| entry.starts_with("alert")));
}

#[test]
fn a_denied_system_audio_permission_warns_at_start_and_the_meeting_records() {
    let mut h = working();
    h.coordinator.backend.system_audio_denied = true;
    h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(
        h.log.take(),
        [
            "start mic",
            "start system",
            "alert SystemAudioDenied",
            "recording Both"
        ]
    );
    h.coordinator.stop().unwrap();

    // The microphone alone does not use system audio.
    h.log.take();
    h.coordinator.start(MeetingCaptureMode::MicOnly).unwrap();
    assert!(!h.log.take().iter().any(|entry| entry.starts_with("alert")));
}

#[test]
fn a_second_start_is_refused_while_recording() {
    let mut h = working();
    h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(
        h.coordinator.start(MeetingCaptureMode::MicOnly),
        Err(MeetingError::AlreadyRecording)
    );
    assert_eq!(h.store.list(None, 10).unwrap().meetings.len(), 1);
    h.coordinator.stop().unwrap();
    assert_eq!(h.coordinator.stop(), Err(MeetingError::NotRecording));
}

#[test]
fn each_mode_opens_only_its_sources() {
    let mut h = working();
    h.coordinator.start(MeetingCaptureMode::MicOnly).unwrap();
    h.coordinator.stop().unwrap();
    h.coordinator.start(MeetingCaptureMode::SystemOnly).unwrap();
    h.coordinator.stop().unwrap();
    let started: Vec<_> = h
        .log
        .take()
        .into_iter()
        .filter(|entry| entry.starts_with("start"))
        .collect();
    assert_eq!(started, ["start mic", "start system"]);
}

#[test]
fn without_system_audio_support_every_mode_records_the_microphone() {
    let mut h = harness(SystemAudioSupport::OsTooOld, true, true);
    h.coordinator.start(MeetingCaptureMode::SystemOnly).unwrap();
    h.coordinator.stop().unwrap();
    assert_eq!(h.log.take()[..2], ["start mic", "recording Mic"]);
    assert_eq!(statuses(&h.store)[0].1, MeetingTracks::Mic);
}

#[test]
fn both_mode_continues_with_the_source_that_started() {
    let mut h = harness(SystemAudioSupport::Available, true, false);
    h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(
        h.log.take(),
        ["start mic", "alert SystemAudioUnavailable", "recording Mic"]
    );
    assert_eq!(statuses(&h.store)[0].1, MeetingTracks::Mic);

    let mut h = harness(SystemAudioSupport::Available, false, true);
    h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(
        h.log.take(),
        [
            "start system",
            "alert MicrophoneUnavailable",
            "recording System"
        ]
    );
}

#[test]
fn a_single_source_mode_aborts_when_its_source_fails() {
    let mut h = harness(SystemAudioSupport::Available, true, false);
    let error = h.coordinator.start(MeetingCaptureMode::SystemOnly);
    assert_eq!(
        error,
        Err(MeetingError::NoSource("system unavailable".to_string()))
    );
    assert_eq!(h.coordinator.runtime(), MeetingRuntime::Idle);
    assert!(h.store.list(None, 10).unwrap().meetings.is_empty());
    assert_eq!(std::fs::read_dir(&h.root).unwrap().count(), 0);
    assert!(h.log.take().is_empty());
}

#[test]
fn discarding_deletes_the_recording() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.discard(id).unwrap();
    assert_eq!(h.coordinator.runtime(), MeetingRuntime::Idle);
    assert!(h.store.list(None, 10).unwrap().meetings.is_empty());
    assert!(!paths::meeting_dir(&h.root, id).exists());
    assert_eq!(std::fs::read_dir(&h.root).unwrap().count(), 0);
    assert!(!h.trash_folder().exists());
    assert_eq!(h.coordinator.discard(id), Err(MeetingError::NotRecording));
}

#[test]
fn a_discard_aimed_at_an_ended_meeting_keeps_the_one_recording_now() {
    let mut h = working();
    let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    let second = h.coordinator.start(MeetingCaptureMode::Both).unwrap();

    assert_eq!(
        h.coordinator.discard(first),
        Err(MeetingError::NotRecording)
    );
    assert!(matches!(
        h.coordinator.runtime(),
        MeetingRuntime::Recording { id, .. } if id == second
    ));
    assert!(paths::meeting_dir(&h.root, second).exists());
}

#[test]
fn the_cap_warns_once_then_stops_and_keeps_the_meeting() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();

    h.clock.advance(Duration::from_secs(2 * 3600 + 54 * 60));
    assert!(h.coordinator.tick(id));
    assert!(h.log.take().is_empty());

    h.clock.advance(Duration::from_secs(60));
    assert!(h.coordinator.tick(id));
    h.clock.advance(Duration::from_secs(1));
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), ["alert StopsSoon"]);

    h.clock.advance(Duration::from_secs(5 * 60));
    assert!(!h.coordinator.tick(id));
    assert_eq!(
        h.log.take(),
        [
            "finish mic 10801s",
            "finish system 10801s",
            "idle",
            "recorded",
            "alert StoppedAtLimit"
        ]
    );
    assert_eq!(statuses(&h.store)[0].0, MeetingStatus::Processing);
    assert!(!h.coordinator.tick(id));
}

#[test]
fn the_meeting_stops_at_exactly_three_hours_as_one_recording() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.clock.advance(CAP - Duration::from_secs(1));
    assert!(h.coordinator.tick(id));
    h.log.take();

    h.clock.advance(Duration::from_secs(1));
    assert!(!h.coordinator.tick(id));

    assert_eq!(
        h.log.take(),
        [
            "finish mic 10800s",
            "finish system 10800s",
            "idle",
            "recorded",
            "alert StoppedAtLimit"
        ]
    );
    assert_eq!(h.coordinator.runtime(), MeetingRuntime::Idle);
    assert_eq!(
        statuses(&h.store),
        [(MeetingStatus::Processing, MeetingTracks::Both, 10_800_000)]
    );
    let folders = std::fs::read_dir(&h.root).unwrap().count();
    assert_eq!(folders, 1, "no folder for a second recording");
}

#[test]
fn a_disk_nearly_full_while_recording_lets_the_meeting_go_on() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    h.clock.advance(Duration::from_secs(1));
    h.coordinator.backend.free_bytes = Some(0);
    assert!(h.coordinator.tick(id), "only a failed write stops it");
    assert_eq!(h.log.take(), Vec::<String>::new());
}

#[test]
fn a_meeting_is_on_headphones_only_when_it_never_played_on_speakers() {
    let mut h = working();
    h.coordinator.backend.route = OutputRoute::Headphones;
    let worn = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.clock.advance(Duration::from_secs(60));
    assert!(h.coordinator.tick(worn));
    h.coordinator.stop().unwrap();
    assert_eq!(h.store.output_route(worn).unwrap(), OutputRoute::Headphones);

    // Headphones taken off for a minute, then put back.
    let mixed = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.backend.route = OutputRoute::Speakers;
    h.clock.advance(Duration::from_secs(60));
    assert!(h.coordinator.tick(mixed));
    h.coordinator.backend.route = OutputRoute::Headphones;
    h.clock.advance(Duration::from_secs(60));
    assert!(h.coordinator.tick(mixed));
    h.coordinator.stop().unwrap();
    assert_eq!(h.store.output_route(mixed).unwrap(), OutputRoute::Speakers);
}

#[test]
fn time_asleep_counts_toward_the_cap_but_not_into_the_audio() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();

    h.clock.advance(Duration::from_secs(3600));
    h.clock.sleep(Duration::from_secs(2 * 3600));
    let woke = h.clock.unix_ms();
    assert!(!h.coordinator.tick(id));
    assert_eq!(
        h.log.take(),
        [
            format!(
                "alert {:?}",
                MeetingAlert::PausedWhileAsleep {
                    from_ms: woke - 2 * 3_600_000,
                    to_ms: woke
                }
            ),
            "finish mic 3600s".to_string(),
            "finish system 3600s".to_string(),
            "idle".to_string(),
            "recorded".to_string(),
            "alert StoppedAtLimit".to_string()
        ]
    );
    assert_eq!(h.store.slept_ms(id).unwrap(), 2 * 3_600_000);
}

#[test]
fn the_mac_is_kept_awake_while_a_meeting_records_and_only_then() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(h.awake.take(), ["awake on"]);
    assert!(h.coordinator.tick(id));
    assert!(h.awake.take().is_empty());
    h.coordinator.stop().unwrap();
    assert_eq!(h.awake.take(), ["awake off"]);

    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.discard(id).unwrap();
    assert_eq!(h.awake.take(), ["awake on", "awake off"]);

    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.failed.store(true, Ordering::SeqCst);
    assert!(!h.coordinator.tick(id));
    assert_eq!(h.awake.take(), ["awake on", "awake off"]);
}

#[test]
fn a_meeting_that_cannot_start_never_keeps_the_mac_awake() {
    let mut h = harness(SystemAudioSupport::Available, false, false);
    assert!(h.coordinator.start(MeetingCaptureMode::Both).is_err());
    assert!(h.awake.take().is_empty());
}

#[test]
fn each_sleep_during_a_meeting_is_told_once_with_its_times() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    let paused = |from_ms: i64, to_ms: i64| {
        format!(
            "alert {:?}",
            MeetingAlert::PausedWhileAsleep { from_ms, to_ms }
        )
    };

    h.clock.advance(Duration::from_secs(60));
    assert!(h.coordinator.tick(id));
    h.clock.advance(Duration::from_secs(1));
    h.clock.sleep(Duration::from_secs(20 * 60));
    let woke = h.clock.unix_ms();
    assert!(h.coordinator.tick(id));
    h.clock.advance(Duration::from_secs(1));
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), [paused(woke - 20 * 60_000, woke)]);
    assert_eq!(h.store.slept_ms(id).unwrap(), 20 * 60_000);

    // Another sleep is news again, and adds to the time asleep.
    h.clock.advance(Duration::from_secs(1));
    h.clock.sleep(Duration::from_secs(30));
    let woke = h.clock.unix_ms();
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), [paused(woke - 30_000, woke)]);
    assert_eq!(h.store.slept_ms(id).unwrap(), 20 * 60_000 + 30_000);

    // The audio holds only the time awake.
    h.coordinator.stop().unwrap();
    assert_eq!(h.log.take()[0], "finish mic 63s");
}

#[test]
fn a_clock_correction_of_a_few_seconds_is_not_a_sleep() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    h.clock.advance(Duration::from_secs(1));
    h.clock.sleep(Duration::from_secs(2));
    assert!(h.coordinator.tick(id));
    assert!(h.log.take().is_empty());
    assert_eq!(h.store.slept_ms(id).unwrap(), 0);
}

#[test]
fn audio_dropped_past_two_seconds_is_told_once_and_the_meeting_records_on() {
    let mut h = working();
    // 1.8 s over both tracks: not worth a notification.
    h.coordinator.backend.dropped_ms = 900;
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    assert!(h.coordinator.tick(id));
    assert!(h.log.take().is_empty());
    h.coordinator.stop().unwrap();

    h.coordinator.backend.dropped_ms = 2_500;
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    assert!(h.coordinator.tick(id));
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), ["alert AudioDropped"]);
}

#[test]
fn a_write_failure_stops_the_meeting_and_keeps_what_was_recorded() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    h.clock.advance(Duration::from_secs(30));
    h.failed.store(true, Ordering::SeqCst);

    assert!(!h.coordinator.tick(id));

    assert_eq!(
        h.log.take(),
        [
            "finish mic 30s",
            "finish system 30s",
            "idle",
            "recorded",
            "alert WriteFailed"
        ]
    );
    assert_eq!(statuses(&h.store)[0].0, MeetingStatus::Processing);
}

#[test]
fn lost_system_audio_warns_once_and_the_microphone_keeps_recording() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    h.system_interrupted.store(true, Ordering::SeqCst);

    assert!(h.coordinator.tick(id));
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), ["alert SystemAudioInterrupted"]);

    // Back, then lost again: worth telling again.
    h.system_interrupted.store(false, Ordering::SeqCst);
    assert!(h.coordinator.tick(id));
    h.system_interrupted.store(true, Ordering::SeqCst);
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), ["alert SystemAudioInterrupted"]);
}

#[test]
fn a_lost_microphone_warns_once_and_the_meeting_keeps_recording() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.log.take();
    h.mic_interrupted.store(true, Ordering::SeqCst);

    assert!(h.coordinator.tick(id));
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), ["alert MicrophoneInterrupted"]);

    // System audio dropping as well is news of its own.
    h.system_interrupted.store(true, Ordering::SeqCst);
    assert!(h.coordinator.tick(id));
    assert_eq!(h.log.take(), ["alert SystemAudioInterrupted"]);
}

#[test]
fn a_new_meeting_never_inherits_files_left_under_its_id() {
    let mut h = working();
    // A folder whose deletion failed, under the id the next meeting gets.
    let stale = paths::meeting_dir(&h.root, 1);
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("analysis.json"), "{}").unwrap();

    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();

    assert_eq!(id, 1);
    assert!(paths::meeting_dir(&h.root, id).is_dir());
    assert!(!stale.join("analysis.json").exists());
}

#[cfg(unix)]
#[test]
fn a_stale_folder_that_cannot_be_deleted_never_blocks_a_new_meeting() {
    let mut h = working();
    // Under the id the next meeting gets, an entry the safe delete
    // refuses to remove.
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(outside.path(), paths::meeting_dir(&h.root, 1)).unwrap();

    let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert_eq!(first, 1);
    let dir = std::fs::symlink_metadata(paths::meeting_dir(&h.root, first)).unwrap();
    assert!(dir.is_dir() && !dir.file_type().is_symlink());
    h.coordinator.discard(first).unwrap();

    // A discarded id is never given again, so the next start is clear.
    let next = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert!(next > first);
    assert!(paths::meeting_dir(&h.root, next).is_dir());
    assert!(outside.path().join("keep.txt").exists());
}

#[cfg(unix)]
#[test]
fn a_meeting_whose_folder_cannot_be_deleted_stays_listed_to_delete_again() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    // A folder the safe delete refuses to remove.
    let dir = paths::meeting_dir(&h.root, id);
    std::fs::remove_dir_all(&dir).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), &dir).unwrap();

    let bin = FolderTrash::new(&h.trash_folder());
    assert!(h.coordinator.delete(id, &notes(), &bin).is_err());
    assert!(h.store.get(id).unwrap().is_some());

    std::fs::remove_file(&dir).unwrap();
    std::fs::create_dir(&dir).unwrap();
    h.coordinator.delete(id, &notes(), &bin).unwrap();
    assert!(h.store.get(id).unwrap().is_none());
    assert!(!dir.exists());
}

#[test]
fn deleting_a_meeting_moves_its_folder_to_the_trash_and_removes_its_rows() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    let dir = paths::meeting_dir(&h.root, id);
    std::fs::write(dir.join(paths::AUDIO_FILE), b"audio").unwrap();
    let bin = FolderTrash::new(&h.trash_folder());

    h.coordinator.delete(id, &notes(), &bin).unwrap();

    let folder = bin.bin.join("2026-10-17 Q4 plan");
    assert!(folder.join("2026-10-17 Q4 plan.md").is_file());
    assert_eq!(
        std::fs::read(folder.join(paths::AUDIO_FILE)).unwrap(),
        b"audio"
    );
    assert!(h.store.get(id).unwrap().is_none());
    assert!(!dir.exists());
    assert_eq!(std::fs::read_dir(&h.root).unwrap().count(), 0);
}

#[test]
fn a_meeting_whose_audio_retention_removed_still_trashes_its_notes() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    paths::remove_meeting_files(&h.root, id, &paths::AUDIO_FILES).unwrap();
    let bin = FolderTrash::new(&h.trash_folder());

    h.coordinator.delete(id, &notes(), &bin).unwrap();

    let folder = bin.bin.join("2026-10-17 Q4 plan");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    assert!(folder.join("2026-10-17 Q4 plan.md").is_file());
    assert!(h.store.get(id).unwrap().is_none());
}

#[test]
fn a_failing_trash_leaves_the_meeting_listed_with_its_folder_and_rows() {
    let mut h = working();
    let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    let dir = paths::meeting_dir(&h.root, id);
    std::fs::write(dir.join(paths::AUDIO_FILE), b"audio").unwrap();
    let before = h.store.get(id).unwrap().unwrap();
    let mut bin = FolderTrash::new(&h.trash_folder());
    bin.fails = true;

    assert!(h.coordinator.delete(id, &notes(), &bin).is_err());

    assert_eq!(h.store.get(id).unwrap().unwrap(), before);
    assert_eq!(
        std::fs::read(dir.join(paths::AUDIO_FILE)).unwrap(),
        b"audio"
    );
    assert!(!dir.join("2026-10-17 Q4 plan.md").exists());
    assert_eq!(std::fs::read_dir(&h.root).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(&bin.bin).unwrap().count(), 0);
}

#[test]
fn an_old_ticker_does_not_drive_a_newer_meeting() {
    let mut h = working();
    let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    let second = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    assert!(!h.coordinator.tick(first));
    assert!(h.coordinator.tick(second));
}

#[test]
fn the_recording_meeting_cannot_be_deleted() {
    let mut h = working();
    let old = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    h.coordinator.stop().unwrap();
    let current = h.coordinator.start(MeetingCaptureMode::Both).unwrap();

    let bin = FolderTrash::new(&h.trash_folder());
    assert_eq!(
        h.coordinator.delete(current, &notes(), &bin),
        Err(MeetingError::InUse)
    );
    assert_eq!(std::fs::read_dir(&bin.bin).unwrap().count(), 0);
    h.coordinator.delete(old, &notes(), &bin).unwrap();

    let ids: Vec<_> = h
        .store
        .list(None, 10)
        .unwrap()
        .meetings
        .iter()
        .map(|meeting| meeting.id)
        .collect();
    assert_eq!(ids, [current]);
    assert!(!paths::meeting_dir(&h.root, old).exists());
}

#[test]
fn the_runtime_tells_the_frontend_when_the_cap_stops_the_meeting() {
    let mut h = working();
    h.coordinator.start(MeetingCaptureMode::Both).unwrap();
    let value = serde_json::to_value(h.coordinator.runtime()).unwrap();
    assert_eq!(value["state"], "recording");
    assert_eq!(value["tracks"], "both");
    assert_eq!(
        value["stops_at_ms"].as_i64().unwrap() - value["started_at_ms"].as_i64().unwrap(),
        3 * 3600 * 1000
    );
    assert_eq!(
        serde_json::to_value(MeetingRuntime::Idle).unwrap(),
        serde_json::json!({ "state": "idle" })
    );
}
