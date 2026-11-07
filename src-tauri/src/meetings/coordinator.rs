//! One meeting at a time: start, stop, discard and the 3-hour cap. Devices,
//! notifications and time come in through traits, so the rules are tested
//! without audio hardware or a running app.

use log::{error, info, warn};
use serde::Serialize;
use specta::Type;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::cap::{cap_action, slept, CapAction, CAP};
use super::capture::route::OutputRoute;
use super::capture::system::SystemAudioSupport;
use super::live::{LiveMeeting, LiveTap};
use super::merge::Track as Source;
use super::paths;
use super::store::{MeetingTracks, MeetingsStore};
use super::trash;
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
    /// Hands what the track writes from now on to the live transcript too.
    fn live(&self, _tap: LiveTap) {}
    /// Stops the source and pads the file to `duration`. Returns whether
    /// another app played sound while it recorded, which only system audio
    /// can tell.
    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String>;
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
    /// Where the Mac plays sound now.
    fn output_route(&self) -> OutputRoute;
    /// Starts the live transcript of meeting `id`, recording `sources` in
    /// `dir`. `None` when it does not run.
    fn live(
        &self,
        _id: i64,
        _dir: &Path,
        _sources: &[Source],
        _route: OutputRoute,
    ) -> Option<LiveMeeting> {
        None
    }
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
    StopsSoon,
    StoppedAtLimit,
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
        id: i64,
        started_at_ms: i64,
        /// When the 3-hour cap stops the meeting.
        stops_at_ms: i64,
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
    id: i64,
    started_at_ms: i64,
    origin: Instant,
    tracks: Vec<(Source, Box<dyn Track>)>,
    recorded: MeetingTracks,
    warned: bool,
    /// The sources lost at the last tick, which the user was told about.
    interrupted: Vec<Source>,
    /// `Instant` and wall-clock time of the last tick, which tell a sleep.
    last_tick: (Instant, i64),
    /// The user was told the tracks dropped audio.
    dropped_told: bool,
    /// Where this meeting played sound, over all of it, and at the last tick.
    route: OutputRoute,
    playing_on: OutputRoute,
    /// Keeps the Mac awake until the meeting ends, however it ends.
    _awake: Option<Box<dyn Send>>,
    live: Option<LiveMeeting>,
}

/// A recording whose end the database refused, on a full disk.
struct Unclosed {
    id: i64,
    ended_at_ms: i64,
    duration_ms: i64,
}

pub struct Coordinator<B, N, C> {
    backend: B,
    notifier: N,
    clock: C,
    store: Arc<MeetingsStore>,
    root: PathBuf,
    active: Option<Active>,
    /// Tried again at each tick of their meeting until saved.
    unclosed: Vec<Unclosed>,
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
            unclosed: Vec::new(),
        }
    }

    pub fn runtime(&self) -> MeetingRuntime {
        match &self.active {
            None => MeetingRuntime::Idle,
            Some(active) => MeetingRuntime::Recording {
                id: active.id,
                started_at_ms: active.started_at_ms,
                stops_at_ms: active.started_at_ms + CAP.as_millis() as i64,
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
        let route = self.backend.output_route();
        info!("Meeting {id}: sound plays on {route:?}");
        let sources: Vec<Source> = tracks.iter().map(|(source, _)| *source).collect();
        let live = self.backend.live(id, &dir, &sources, route);
        if let Some(live) = &live {
            for (source, track) in &tracks {
                track.live(live.tap(*source));
            }
        }
        self.active = Some(Active {
            id,
            started_at_ms,
            origin,
            tracks,
            recorded,
            warned: false,
            interrupted: Vec::new(),
            last_tick: (origin, started_at_ms),
            dropped_told: false,
            route,
            playing_on: route,
            _awake: self.backend.keep_awake(),
            live,
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
        drop(active.live);
        if played {
            if let Err(error) = self.store.set_system_played(active.id) {
                error!(
                    "Meeting {}: cannot note system playback: {error}",
                    active.id
                );
            }
        }
        if let Err(error) = self.store.set_output_route(active.id, active.route) {
            error!(
                "Meeting {}: cannot note where sound played: {error}",
                active.id
            );
        }
        if dropped_ms > 0 {
            if let Err(error) = self.store.set_dropped_ms(active.id, dropped_ms) {
                error!(
                    "Meeting {}: cannot note the dropped audio: {error}",
                    active.id
                );
            }
        }
        let unclosed = Unclosed {
            id: active.id,
            ended_at_ms: self.clock.unix_ms(),
            duration_ms: duration.as_millis() as i64,
        };
        let result = self
            .store
            .finish_recording(unclosed.id, unclosed.ended_at_ms, unclosed.duration_ms)
            .map_err(storage);
        self.notifier.runtime_changed(&self.runtime());
        match &result {
            Ok(()) => self.notifier.recorded(active.id),
            Err(error) => {
                error!("Meeting {}: cannot save its end: {error}", active.id);
                self.unclosed.push(unclosed);
            }
        }
        result.map(|()| active.id)
    }

    fn is_unclosed(&self, id: i64) -> bool {
        self.unclosed.iter().any(|unclosed| unclosed.id == id)
    }

    /// Saves the end of meeting `id` the database refused. Returns whether
    /// it is still unsaved.
    fn retry_close(&mut self, id: i64) -> bool {
        let Some(at) = self.unclosed.iter().position(|unclosed| unclosed.id == id) else {
            return false;
        };
        let unclosed = &self.unclosed[at];
        if self
            .store
            .finish_recording(id, unclosed.ended_at_ms, unclosed.duration_ms)
            .is_err()
        {
            return true;
        }
        self.unclosed.remove(at);
        info!("Meeting {id}: its end is saved");
        self.notifier.recorded(id);
        false
    }

    /// Ends recording `id` and deletes it. Any other meeting, recording or
    /// not, is left alone: the request may come from a dialog opened for a
    /// meeting that has ended since.
    pub fn discard(&mut self, id: i64) -> Result<(), MeetingError> {
        let active = self
            .active
            .take_if(|active| active.id == id)
            .ok_or(MeetingError::NotRecording)?;
        if let Some(live) = active.live {
            live.discard();
        }
        for (_, track) in active.tracks {
            let _ = track.finish(Duration::ZERO);
        }
        self.forget(active.id);
        self.notifier.runtime_changed(&self.runtime());
        Ok(())
    }

    /// Called about once a second while meeting `id` records. Returns
    /// whether to call it again: the meeting records, or its end waits to
    /// be saved.
    pub fn tick(&mut self, id: i64) -> bool {
        if self.is_unclosed(id) {
            return self.retry_close(id);
        }
        let Some(active) = self.active.as_mut().filter(|active| active.id == id) else {
            return false;
        };
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
            return self.is_unclosed(id);
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
        let playing_on = self.backend.output_route();
        if playing_on != active.playing_on {
            info!("Meeting {id}: sound now plays on {playing_on:?}");
            active.playing_on = playing_on;
            if let Some(live) = &active.live {
                live.set_headphones(playing_on == OutputRoute::Headphones);
            }
        }
        active.route = active.route.and(playing_on);
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
        // the clock being set back.
        let awake = self.clock.now().saturating_duration_since(active.origin);
        let wall = u64::try_from(self.clock.unix_ms() - active.started_at_ms)
            .map_or(Duration::ZERO, Duration::from_millis);
        match cap_action(awake.max(wall), active.warned) {
            CapAction::None => true,
            CapAction::Warn => {
                active.warned = true;
                self.notifier.alert(MeetingAlert::StopsSoon);
                true
            }
            CapAction::Stop => {
                let _ = self.stop();
                self.notifier.alert(MeetingAlert::StoppedAtLimit);
                self.is_unclosed(id)
            }
        }
    }

    /// A person's Delete: the folder and `notes` go to the Trash, then the
    /// rows go.
    pub fn delete(
        &mut self,
        id: i64,
        notes: &trash::Notes,
        bin: &dyn trash::Trash,
    ) -> Result<(), MeetingError> {
        if self.active.as_ref().is_some_and(|active| active.id == id) {
            return Err(MeetingError::InUse);
        }
        // The folder first: a meeting whose audio stays on disk stays listed,
        // so the user can delete it again.
        trash::trash_meeting(&self.root, id, notes, bin).map_err(storage)?;
        self.store.delete(id).map_err(storage)?;
        self.unclosed.retain(|unclosed| unclosed.id != id);
        Ok(())
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
mod tests;
