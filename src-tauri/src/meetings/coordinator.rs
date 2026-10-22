//! One meeting at a time: start, stop, discard, a new part every 3 hours of
//! audio and the 12-hour cap. Devices,
//! notifications and time come in through traits, so the rules are tested
//! without audio hardware or a running app.

use log::{error, info, warn};
use serde::Serialize;
use specta::Type;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cap::{cap_action, part_length, slept, CapAction};
use super::capture::system::SystemAudioSupport;
use super::merge::Track as Source;
use super::paths;
use super::store::{MeetingTracks, MeetingsStore};
use crate::settings::MeetingCaptureMode;

/// One source being recorded to its WAV.
pub trait Track: Send {
    /// The track can no longer be written.
    fn failed(&self) -> bool;
    /// The source stopped delivering audio and could not be brought back
    /// yet; the track records silence meanwhile.
    fn interrupted(&self) -> bool;
    /// Audio the source delivered faster than the disk took it, recorded as
    /// silence instead.
    fn dropped_ms(&self) -> u64;
    /// Ends the current file at `origin`, the start of the next part, and
    /// records on into the same file name in `dir`. The source stays open.
    fn rotate(&self, dir: &Path, origin: Instant) -> Rotation;
    /// Stops the source and pads the file to `duration`. Returns whether
    /// another app played sound while it recorded, which only system audio
    /// can tell.
    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String>;
}

/// A track moving on to the next part's file. Every track is asked first and
/// waited for after, so they all cut at the same moment.
pub struct Rotation {
    closed: Receiver<Result<(), String>>,
    played: bool,
}

impl Rotation {
    /// `closed` answers once the previous file is closed; `played` is
    /// whether another app played sound during it.
    pub fn new(closed: Receiver<Result<(), String>>, played: bool) -> Self {
        Self { closed, played }
    }

    /// Waits until the previous file is closed. Returns whether another app
    /// played sound during it, as `Track::finish` does.
    pub fn wait(self) -> Result<bool, String> {
        self.closed
            .recv()
            .unwrap_or_else(|_| Err("The track writer stopped".to_string()))
            .map(|()| self.played)
    }
}

pub trait CaptureBackend: Send {
    fn system_audio(&self) -> SystemAudioSupport;
    fn start_mic(&self, dir: &Path, origin: Instant) -> Result<Box<dyn Track>, String>;
    fn start_system(&self, dir: &Path, origin: Instant) -> Result<Box<dyn Track>, String>;
    /// Bytes free for new files on the volume of `dir`, when it can be read.
    fn free_bytes(&self, dir: &Path) -> Option<u64>;
    /// macOS refuses Murmur the microphone, which then gives only silence.
    fn microphone_denied(&self) -> bool;
    /// macOS refuses Murmur System Audio Recording: the tap still starts,
    /// but records only silence.
    fn system_audio_denied(&self) -> bool;
    /// Keeps the Mac from sleeping on its own until the returned guard is
    /// dropped. `None` when macOS refuses.
    fn keep_awake(&self) -> Option<Box<dyn Send>>;
}

/// Below this much free space a meeting warns at start: three hours of both
/// tracks take about 700 MB before they are compressed.
pub const LOW_DISK_BYTES: u64 = 1_500_000_000;

/// Audio the tracks dropped, summed, past which the user is told once.
const DROPPED_ALERT_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingAlert {
    /// Both sources were asked for and only the microphone started.
    SystemAudioUnavailable,
    /// Both sources were asked for and only system audio started.
    MicrophoneUnavailable,
    StartFailed,
    /// The meeting stops in 5 minutes, at 12 hours.
    StopsSoon,
    StoppedAtLimit,
    /// The meeting was due for its next part with the disk nearly full, so
    /// it stopped and kept what was recorded.
    StoppedLowDisk,
    WriteFailed,
    /// System audio stopped after the sound output changed; the microphone
    /// keeps recording.
    SystemAudioInterrupted,
    /// The microphone stopped (unplugged, or a headset dropped out) and could
    /// not be reopened yet; the other source keeps recording.
    MicrophoneInterrupted,
    /// The microphone permission is off, so the microphone track records
    /// silence; only Privacy & Security can turn it back on.
    MicrophoneDenied,
    /// The System Audio Recording permission is off, so the system track
    /// records silence; only Privacy & Security can turn it back on.
    SystemAudioDenied,
    /// The meeting records, but the disk may fill before it ends.
    LowDiskSpace,
    /// The Mac slept anyway (lid closed, battery low): nothing was recorded
    /// between these two times.
    PausedWhileAsleep {
        from_ms: i64,
        to_ms: i64,
    },
    /// The track writers fell behind for more than `DROPPED_ALERT_MS`; that
    /// audio is silence in the recording.
    AudioDropped,
}

pub trait Notifier: Send {
    fn runtime_changed(&self, runtime: &MeetingRuntime);
    fn alert(&self, alert: MeetingAlert);
    /// Meeting `id` was recorded and waits for processing.
    fn recorded(&self, id: i64);
}

pub trait Clock: Send {
    fn now(&self) -> Instant;
    fn unix_ms(&self) -> i64;
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MeetingRuntime {
    Idle,
    Recording {
        /// The part recording now.
        id: i64,
        /// When this part started.
        started_at_ms: i64,
        /// When the first part started, which the pill counts from.
        meeting_started_at_ms: i64,
        tracks: MeetingTracks,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetingError {
    AlreadyRecording,
    NotRecording,
    /// The meeting is being recorded, so it cannot be deleted.
    InUse,
    /// No source could be opened.
    NoSource(String),
    Storage(String),
}

impl fmt::Display for MeetingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRecording => formatter.write_str("A meeting is already being recorded"),
            Self::NotRecording => formatter.write_str("No meeting is being recorded"),
            Self::InUse => formatter.write_str("This meeting is being recorded"),
            Self::NoSource(reason) => write!(formatter, "Cannot record the meeting: {reason}"),
            Self::Storage(reason) => write!(formatter, "Cannot save the meeting: {reason}"),
        }
    }
}

fn storage(error: impl fmt::Display) -> MeetingError {
    MeetingError::Storage(error.to_string())
}

struct Active {
    /// The id of the first part, which the ticker drives the meeting by.
    started_as: i64,
    /// The part recording now.
    id: i64,
    started_at_ms: i64,
    origin: Instant,
    meeting_started_at_ms: i64,
    meeting_origin: Instant,
    /// Audio the tracks had dropped when this part started: they count over
    /// the whole meeting.
    dropped_before: u64,
    tracks: Vec<(Source, Box<dyn Track>)>,
    recorded: MeetingTracks,
    warned: bool,
    /// The sources lost at the last tick, which the user was told about.
    interrupted: Vec<Source>,
    /// `Instant` and wall-clock time of the last tick, which tell a sleep.
    last_tick: (Instant, i64),
    /// The user was told the tracks dropped audio.
    dropped_told: bool,
    /// Keeps the Mac awake until the meeting ends, however it ends.
    _awake: Option<Box<dyn Send>>,
}

pub struct Coordinator<B, N, C> {
    backend: B,
    notifier: N,
    clock: C,
    store: Arc<MeetingsStore>,
    root: PathBuf,
    active: Option<Active>,
    /// The audio of a part, past which the meeting goes on in the next one.
    part_length: Duration,
}

impl<B: CaptureBackend, N: Notifier, C: Clock> Coordinator<B, N, C> {
    pub fn new(
        backend: B,
        notifier: N,
        clock: C,
        store: Arc<MeetingsStore>,
        root: PathBuf,
    ) -> Self {
        Self {
            backend,
            notifier,
            clock,
            store,
            root,
            active: None,
            part_length: part_length(),
        }
    }

    pub fn runtime(&self) -> MeetingRuntime {
        match &self.active {
            None => MeetingRuntime::Idle,
            Some(active) => MeetingRuntime::Recording {
                id: active.id,
                started_at_ms: active.started_at_ms,
                meeting_started_at_ms: active.meeting_started_at_ms,
                tracks: active.recorded,
            },
        }
    }

    /// Without system audio support, every mode records the microphone.
    fn wanted_sources(&self, mode: MeetingCaptureMode) -> (bool, bool) {
        if self.backend.system_audio() != SystemAudioSupport::Available {
            return (true, false);
        }
        match mode {
            MeetingCaptureMode::Both => (true, true),
            MeetingCaptureMode::SystemOnly => (false, true),
            MeetingCaptureMode::MicOnly => (true, false),
        }
    }

    pub fn start(&mut self, mode: MeetingCaptureMode) -> Result<i64, MeetingError> {
        if self.active.is_some() {
            return Err(MeetingError::AlreadyRecording);
        }
        let (want_mic, want_system) = self.wanted_sources(mode);
        let requested =
            MeetingTracks::from_parts(want_mic, want_system).unwrap_or(MeetingTracks::Mic);
        let started_at_ms = self.clock.unix_ms();
        let origin = self.clock.now();
        let id = self
            .store
            .insert_recording(started_at_ms, requested)
            .map_err(storage)?;
        let dir = match paths::fresh_meeting_dir(&self.root, id, started_at_ms) {
            Ok(dir) => dir,
            Err(error) => {
                // Nothing of this meeting is on disk: whatever is left under
                // its id belongs to no meeting.
                if let Err(error) = self.store.delete(id) {
                    error!("Meeting {id}: cannot delete its row: {error}");
                }
                return Err(storage(error));
            }
        };

        let mut tracks = Vec::new();
        let mut reasons = Vec::new();
        let mut open =
            |source: Source, wanted: bool, result: &dyn Fn() -> Result<Box<dyn Track>, String>| {
                if !wanted {
                    return false;
                }
                match result() {
                    Ok(track) => {
                        tracks.push((source, track));
                        true
                    }
                    Err(reason) => {
                        warn!("Meeting {id}: a source did not start: {reason}");
                        reasons.push(reason);
                        false
                    }
                }
            };
        let mic = open(Source::Mic, want_mic, &|| {
            self.backend.start_mic(&dir, origin)
        });
        let system = open(Source::System, want_system, &|| {
            self.backend.start_system(&dir, origin)
        });

        let Some(recorded) = MeetingTracks::from_parts(mic, system) else {
            self.forget(id);
            return Err(MeetingError::NoSource(reasons.join("; ")));
        };
        if recorded != requested {
            if let Err(error) = self.store.set_tracks(id, recorded) {
                error!("Meeting {id}: cannot record which tracks started: {error}");
            }
            self.notifier.alert(if mic {
                MeetingAlert::SystemAudioUnavailable
            } else {
                MeetingAlert::MicrophoneUnavailable
            });
        }
        if mic && self.backend.microphone_denied() {
            warn!("Meeting {id}: the microphone permission is off");
            self.notifier.alert(MeetingAlert::MicrophoneDenied);
        }
        if system && self.backend.system_audio_denied() {
            warn!("Meeting {id}: the System Audio Recording permission is off");
            self.notifier.alert(MeetingAlert::SystemAudioDenied);
        }
        self.active = Some(Active {
            started_as: id,
            id,
            started_at_ms,
            origin,
            meeting_started_at_ms: started_at_ms,
            meeting_origin: origin,
            dropped_before: 0,
            tracks,
            recorded,
            warned: false,
            interrupted: Vec::new(),
            last_tick: (origin, started_at_ms),
            dropped_told: false,
            _awake: self.backend.keep_awake(),
        });
        self.notifier.runtime_changed(&self.runtime());
        if self
            .backend
            .free_bytes(&dir)
            .is_some_and(|free| free < LOW_DISK_BYTES)
        {
            warn!("Meeting {id}: less than 1.5 GB free on disk");
            self.notifier.alert(MeetingAlert::LowDiskSpace);
        }
        Ok(id)
    }

    /// Ends the recording and leaves it waiting for processing.
    pub fn stop(&mut self) -> Result<i64, MeetingError> {
        let active = self.active.take().ok_or(MeetingError::NotRecording)?;
        let duration = self.clock.now().saturating_duration_since(active.origin);
        let mut played = false;
        let mut dropped_ms = 0;
        for (source, track) in active.tracks {
            let dropped = track.dropped_ms();
            dropped_ms += dropped;
            if dropped > 0 {
                warn!(
                    "Meeting {}: {source:?} track dropped {dropped} ms",
                    active.id
                );
            } else {
                info!("Meeting {}: {source:?} track dropped no audio", active.id);
            }
            match track.finish(duration) {
                Ok(heard) => played |= heard,
                Err(error) => error!(
                    "Meeting {}: a track did not close cleanly: {error}",
                    active.id
                ),
            }
        }
        let result = self.close_part(
            active.id,
            duration,
            played,
            dropped_ms.saturating_sub(active.dropped_before),
        );
        self.notifier.runtime_changed(&self.runtime());
        if result.is_ok() {
            self.notifier.recorded(active.id);
        }
        result.map(|()| active.id)
    }

    /// Leaves part `id`, recorded for `duration`, waiting for processing.
    fn close_part(
        &self,
        id: i64,
        duration: Duration,
        played: bool,
        dropped_ms: u64,
    ) -> Result<(), MeetingError> {
        if played {
            if let Err(error) = self.store.set_system_played(id) {
                error!("Meeting {id}: cannot note system playback: {error}");
            }
        }
        if dropped_ms > 0 {
            if let Err(error) = self.store.set_dropped_ms(id, dropped_ms) {
                error!("Meeting {id}: cannot note the dropped audio: {error}");
            }
        }
        self.store
            .finish_recording(id, self.clock.unix_ms(), duration.as_millis() as i64)
            .map_err(storage)
    }

    /// Goes on recording into a new part, linked to the one recording now,
    /// which then waits for processing. The devices stay open, so the seam
    /// loses no audio. Returns whether the meeting still records: when the
    /// next part cannot be made, it stops instead.
    fn roll(&mut self) -> bool {
        let Some(active) = self.active.as_ref() else {
            return false;
        };
        let (previous, recorded) = (active.id, active.recorded);
        let dir = paths::meeting_dir(&self.root, previous);
        if self
            .backend
            .free_bytes(&dir)
            .is_some_and(|free| free < LOW_DISK_BYTES)
        {
            warn!("Meeting {previous}: less than 1.5 GB free, so it stops instead of going on");
            let _ = self.stop();
            self.notifier.alert(MeetingAlert::StoppedLowDisk);
            return false;
        }
        let started_at_ms = self.clock.unix_ms();
        let origin = self.clock.now();
        let next = match self.store.insert_part(started_at_ms, recorded, previous) {
            Ok(next) => next,
            Err(error) => {
                error!("Meeting {previous}: cannot start the next part: {error}");
                let _ = self.stop();
                self.notifier.alert(MeetingAlert::StoppedAtLimit);
                return false;
            }
        };
        let dir = match paths::fresh_meeting_dir(&self.root, next, started_at_ms) {
            Ok(dir) => dir,
            Err(error) => {
                error!("Meeting {next}: cannot create its folder: {error}");
                self.forget(next);
                let _ = self.stop();
                self.notifier.alert(MeetingAlert::StoppedAtLimit);
                return false;
            }
        };
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let rotations: Vec<_> = active
            .tracks
            .iter()
            .map(|(_, track)| track.rotate(&dir, origin))
            .collect();
        let mut played = false;
        for rotation in rotations {
            match rotation.wait() {
                Ok(heard) => played |= heard,
                // The track has failed: the next tick stops the meeting.
                Err(error) => error!("Meeting {previous}: a track did not roll cleanly: {error}"),
            }
        }
        let duration = origin.saturating_duration_since(active.origin);
        let dropped: u64 = active.tracks.iter().map(|(_, t)| t.dropped_ms()).sum();
        let dropped_ms = dropped.saturating_sub(active.dropped_before);
        active.id = next;
        active.started_at_ms = started_at_ms;
        active.origin = origin;
        active.dropped_before = dropped;
        info!(
            "Meeting {previous} goes on as meeting {next} after {} s",
            duration.as_secs()
        );
        let closed = self.close_part(previous, duration, played, dropped_ms);
        self.notifier.runtime_changed(&self.runtime());
        match closed {
            Ok(()) => self.notifier.recorded(previous),
            Err(error) => error!("Meeting {previous}: cannot finish its part: {error}"),
        }
        true
    }

    /// Ends recording `id` and deletes it. Any other meeting, recording or
    /// not, is left alone: the request may come from a dialog opened for a
    /// meeting that has ended since.
    pub fn discard(&mut self, id: i64) -> Result<(), MeetingError> {
        let active = self
            .active
            .take_if(|active| active.id == id)
            .ok_or(MeetingError::NotRecording)?;
        for (_, track) in active.tracks {
            let _ = track.finish(Duration::ZERO);
        }
        self.forget(active.id);
        self.notifier.runtime_changed(&self.runtime());
        Ok(())
    }

    /// Called about once a second while the meeting that started as `id`
    /// records, whichever part it is in. Returns whether it still records.
    pub fn tick(&mut self, started_as: i64) -> bool {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.started_as == started_as)
        else {
            return false;
        };
        let id = active.id;
        // Neither track records while the Mac sleeps, and both place audio
        // by `Instant`, so they stay aligned and leave the time out.
        let now = (self.clock.now(), self.clock.unix_ms());
        let (last_instant, last_ms) = std::mem::replace(&mut active.last_tick, now);
        if let Some(gap) = slept(
            now.0.saturating_duration_since(last_instant),
            now.1 - last_ms,
        ) {
            let gap_ms = gap.as_millis() as u64;
            warn!("Meeting {id}: the Mac slept for {} s", gap.as_secs());
            if let Err(error) = self.store.add_slept_ms(id, gap_ms) {
                error!("Meeting {id}: cannot note the time asleep: {error}");
            }
            self.notifier.alert(MeetingAlert::PausedWhileAsleep {
                from_ms: now.1 - gap_ms as i64,
                to_ms: now.1,
            });
        }
        if active.tracks.iter().any(|(_, track)| track.failed()) {
            let _ = self.stop();
            self.notifier.alert(MeetingAlert::WriteFailed);
            return false;
        }
        let interrupted: Vec<Source> = active
            .tracks
            .iter()
            .filter(|(_, track)| track.interrupted())
            .map(|(source, _)| *source)
            .collect();
        for source in interrupted
            .iter()
            .filter(|s| !active.interrupted.contains(s))
        {
            warn!("Meeting {id}: {source:?} audio stopped arriving");
            self.notifier.alert(match source {
                Source::Mic => MeetingAlert::MicrophoneInterrupted,
                Source::System => MeetingAlert::SystemAudioInterrupted,
            });
        }
        active.interrupted = interrupted;
        // A disk too slow for a moment is not a full one: the meeting goes on.
        let dropped_ms: u64 = active.tracks.iter().map(|(_, t)| t.dropped_ms()).sum();
        if !active.dropped_told && dropped_ms > DROPPED_ALERT_MS {
            active.dropped_told = true;
            warn!("Meeting {id}: the tracks dropped {dropped_ms} ms so far");
            self.notifier.alert(MeetingAlert::AudioDropped);
        }
        // The cap counts wall-clock time, as the pill does: a Mac asleep
        // records nothing, but a meeting left running overnight stops on
        // waking. `Instant` stands still in sleep, so it only guards against
        // the clock being set back. A part counts its audio, which is what
        // sizes its files.
        let now = self.clock.now();
        let awake = now.saturating_duration_since(active.meeting_origin);
        let wall = u64::try_from(self.clock.unix_ms() - active.meeting_started_at_ms)
            .map_or(Duration::ZERO, Duration::from_millis);
        let part = now.saturating_duration_since(active.origin);
        match cap_action(awake.max(wall), part, self.part_length, active.warned) {
            CapAction::None => true,
            CapAction::Roll => self.roll(),
            CapAction::Warn => {
                active.warned = true;
                self.notifier.alert(MeetingAlert::StopsSoon);
                true
            }
            CapAction::Stop => {
                let _ = self.stop();
                self.notifier.alert(MeetingAlert::StoppedAtLimit);
                false
            }
        }
    }

    pub fn delete(&mut self, id: i64) -> Result<(), MeetingError> {
        if self.active.as_ref().is_some_and(|active| active.id == id) {
            return Err(MeetingError::InUse);
        }
        // The folder first: a meeting whose audio stays on disk stays listed,
        // so the user can delete it again.
        paths::remove_meeting_dir(&self.root, id).map_err(storage)?;
        self.store.delete(id).map_err(storage)
    }

    /// Removes a meeting that never got going.
    fn forget(&self, id: i64) {
        if let Err(error) = paths::remove_meeting_dir(&self.root, id) {
            error!("Meeting {id}: cannot delete its folder: {error}");
            return;
        }
        if let Err(error) = self.store.delete(id) {
            error!("Meeting {id}: cannot delete its row: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::store::MeetingStatus;
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
        fn rotate(&self, dir: &Path, _: Instant) -> Rotation {
            assert!(dir.is_dir());
            self.log.push(format!("rotate {}", self.name));
            let (closed, answer) = std::sync::mpsc::sync_channel(1);
            closed.send(Ok(())).unwrap();
            Rotation::new(answer, self.played)
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

    const HOUR: Duration = Duration::from_secs(3600);

    fn recording_id(h: &Harness) -> i64 {
        match h.coordinator.runtime() {
            MeetingRuntime::Recording { id, .. } => id,
            MeetingRuntime::Idle => panic!("not recording"),
        }
    }

    #[test]
    fn three_hours_of_audio_go_on_in_a_linked_part_without_reopening_the_devices() {
        let mut h = working();
        let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        h.log.take();
        h.awake.take();

        h.clock.advance(3 * HOUR - Duration::from_secs(1));
        assert!(h.coordinator.tick(first));
        assert!(h.log.take().is_empty(), "nothing announced at 2:55");
        h.clock.advance(Duration::from_secs(1));
        assert!(h.coordinator.tick(first));

        assert_eq!(
            h.log.take(),
            ["rotate mic", "rotate system", "recording Both", "recorded"]
        );
        assert!(h.awake.take().is_empty(), "the Mac stays awake");
        let second = recording_id(&h);
        assert!(second > first);
        let MeetingRuntime::Recording {
            started_at_ms,
            meeting_started_at_ms,
            ..
        } = h.coordinator.runtime()
        else {
            unreachable!()
        };
        assert_eq!(started_at_ms - meeting_started_at_ms, 3 * 3_600_000);
        let part_one = h.store.get(first).unwrap().unwrap();
        assert_eq!(part_one.status, MeetingStatus::Processing);
        assert_eq!(part_one.duration_ms, 3 * 3_600_000);
        assert_eq!((part_one.part, part_one.previous_id), (1, None));
        let part_two = h.store.get(second).unwrap().unwrap();
        assert_eq!(part_two.status, MeetingStatus::Recording);
        assert_eq!((part_two.part, part_two.previous_id), (2, Some(first)));
        assert!(paths::meeting_dir(&h.root, second).is_dir());

        // The ticker of the first part drives the next one.
        h.clock.advance(Duration::from_secs(60));
        assert!(h.coordinator.tick(first));
        h.coordinator.stop().unwrap();
        assert_eq!(
            h.log.take(),
            ["finish mic 60s", "finish system 60s", "idle", "recorded"]
        );
        assert_eq!(h.store.get(second).unwrap().unwrap().duration_ms, 60_000);
        assert_eq!(h.awake.take(), ["awake off"]);
    }

    #[test]
    fn each_part_keeps_its_own_playback_and_dropped_audio() {
        let mut h = working();
        h.coordinator.backend.system_played = true;
        h.coordinator.backend.dropped_ms = 400;
        let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        h.clock.advance(3 * HOUR);
        assert!(h.coordinator.tick(first));
        let second = recording_id(&h);
        h.coordinator.stop().unwrap();

        assert!(h.store.system_played(first).unwrap());
        assert!(h.store.system_played(second).unwrap());
        assert_eq!(h.store.dropped_ms(first).unwrap(), 800);
        assert_eq!(h.store.dropped_ms(second).unwrap(), 0, "nothing more");
    }

    #[test]
    fn discarding_a_later_part_keeps_the_parts_before() {
        let mut h = working();
        let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        h.clock.advance(3 * HOUR);
        assert!(h.coordinator.tick(first));
        let second = recording_id(&h);

        h.coordinator.discard(second).unwrap();

        assert_eq!(h.coordinator.runtime(), MeetingRuntime::Idle);
        assert!(h.store.get(second).unwrap().is_none());
        assert_eq!(statuses(&h.store)[0].0, MeetingStatus::Processing);
        assert!(!h.coordinator.tick(first));
    }

    #[test]
    fn a_nearly_full_disk_stops_the_meeting_instead_of_starting_a_part() {
        let mut h = working();
        let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        h.log.take();
        h.coordinator.backend.free_bytes = Some(LOW_DISK_BYTES - 1);
        h.clock.advance(3 * HOUR);

        assert!(!h.coordinator.tick(first));

        assert_eq!(
            h.log.take(),
            [
                "finish mic 10800s",
                "finish system 10800s",
                "idle",
                "recorded",
                "alert StoppedLowDisk"
            ]
        );
        assert_eq!(
            statuses(&h.store),
            [(MeetingStatus::Processing, MeetingTracks::Both, 10_800_000)]
        );
    }

    #[test]
    fn the_meeting_is_warned_then_stopped_at_twelve_hours_over_its_parts() {
        let mut h = working();
        let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        for _ in 0..3 {
            h.clock.advance(3 * HOUR);
            assert!(h.coordinator.tick(first));
        }
        h.log.take();

        h.clock.advance(2 * HOUR + Duration::from_secs(55 * 60));
        assert!(h.coordinator.tick(first));
        h.clock.advance(Duration::from_secs(1));
        assert!(h.coordinator.tick(first));
        assert_eq!(h.log.take(), ["alert StopsSoon"]);

        h.clock.advance(Duration::from_secs(5 * 60));
        assert!(!h.coordinator.tick(first));
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
        let parts = statuses(&h.store);
        assert_eq!(parts.len(), 4);
        assert!(parts
            .iter()
            .all(|(status, _, _)| *status == MeetingStatus::Processing));
        assert!(!h.coordinator.tick(first));
    }

    #[test]
    fn a_shorter_part_length_rolls_sooner() {
        let mut h = working();
        h.coordinator.part_length = Duration::from_secs(120);
        let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        h.clock.advance(Duration::from_secs(120));
        assert!(h.coordinator.tick(first));
        assert_ne!(recording_id(&h), first);
    }

    #[test]
    fn time_asleep_counts_toward_the_cap_but_not_into_the_audio() {
        let mut h = working();
        let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        h.log.take();

        h.clock.advance(HOUR);
        h.clock.sleep(11 * HOUR);
        let woke = h.clock.unix_ms();
        assert!(!h.coordinator.tick(id));
        assert_eq!(
            h.log.take(),
            [
                format!(
                    "alert {:?}",
                    MeetingAlert::PausedWhileAsleep {
                        from_ms: woke - 11 * 3_600_000,
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
        assert_eq!(h.store.slept_ms(id).unwrap(), 11 * 3_600_000);
    }

    #[test]
    fn a_sleep_shorter_than_the_cap_neither_rolls_nor_stops_the_meeting() {
        let mut h = working();
        let id = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        h.clock.advance(HOUR);
        h.clock.sleep(5 * HOUR);
        assert!(h.coordinator.tick(id));
        assert_eq!(recording_id(&h), id, "a part counts its audio");
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

        assert!(h.coordinator.delete(id).is_err());
        assert!(h.store.get(id).unwrap().is_some());

        std::fs::remove_file(&dir).unwrap();
        std::fs::create_dir(&dir).unwrap();
        h.coordinator.delete(id).unwrap();
        assert!(h.store.get(id).unwrap().is_none());
        assert!(!dir.exists());
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

        assert_eq!(h.coordinator.delete(current), Err(MeetingError::InUse));
        h.coordinator.delete(old).unwrap();

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
    fn the_runtime_tells_the_frontend_when_the_meeting_and_its_part_started() {
        let mut h = working();
        let first = h.coordinator.start(MeetingCaptureMode::Both).unwrap();
        let value = serde_json::to_value(h.coordinator.runtime()).unwrap();
        assert_eq!(value["state"], "recording");
        assert_eq!(value["tracks"], "both");
        assert_eq!(value["started_at_ms"], value["meeting_started_at_ms"]);
        h.clock.advance(3 * HOUR);
        assert!(h.coordinator.tick(first));
        let value = serde_json::to_value(h.coordinator.runtime()).unwrap();
        assert_eq!(
            value["started_at_ms"].as_i64().unwrap()
                - value["meeting_started_at_ms"].as_i64().unwrap(),
            3 * 3600 * 1000
        );
        assert_eq!(
            serde_json::to_value(MeetingRuntime::Idle).unwrap(),
            serde_json::json!({ "state": "idle" })
        );
    }
}
