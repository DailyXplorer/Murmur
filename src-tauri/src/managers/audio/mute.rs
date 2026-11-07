use crate::settings::get_settings;
use log::debug;

use super::*;

/// The volume control of the default output device.
#[cfg(target_os = "windows")]
pub(super) fn output_volume(
) -> Option<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume> {
    use windows::Win32::Media::Audio::{
        eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    // SAFETY: COM calls on this thread; initializing COM again, or in
    // another apartment already chosen by this thread, leaves it usable.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let devices: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        devices
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .ok()?
            .Activate(CLSCTX_ALL, None)
            .ok()
    }
}

#[cfg(target_os = "windows")]
pub(super) fn set_mute(mute: bool) {
    if let Some(volume) = output_volume() {
        // SAFETY: a plain call on a live COM interface.
        let _ = unsafe { volume.SetMute(mute, std::ptr::null()) };
    }
}

/// Reads the current system output mute state, mirroring `set_mute`. `None`
/// when Windows has no default output device.
#[cfg(target_os = "windows")]
pub(super) fn get_mute() -> Option<bool> {
    let volume = output_volume()?;
    // SAFETY: a plain call on a live COM interface.
    unsafe { volume.GetMute() }
        .ok()
        .map(|muted| muted.as_bool())
}

#[cfg(target_os = "macos")]
pub(super) fn set_mute(mute: bool) {
    use std::process::Command;
    let script = format!(
        "set volume output muted {}",
        if mute { "true" } else { "false" }
    );
    let _ = Command::new("osascript").args(["-e", &script]).output();
}

/// Reads the current system output mute state, mirroring `set_mute`.
///
/// Returns `Some(true)`/`Some(false)` when the state could be determined, or
/// `None` when `osascript` is missing or returns an error. Callers treat `None`
/// as "unknown" and fall back to unmuting on stop,
/// so we never strand the user's audio muted.
#[cfg(target_os = "macos")]
pub(super) fn get_mute() -> Option<bool> {
    use std::process::Command;

    let out = Command::new("osascript")
        .args(["-e", "output muted of (get volume settings)"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    match String::from_utf8_lossy(&out.stdout).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Restores the system mute state after our forced mute, given the state
/// captured just before we muted. We only ever need to unmute — and only when
/// the system was NOT already muted beforehand. If the prior state was muted,
/// we leave it muted (the user's own state). If it's unknown (`None`), we
/// default to unmuting so audio is never left stranded muted by us.
pub(super) fn restore_mute(prev_muted: Option<bool>) {
    if prev_muted != Some(true) {
        set_mute(false);
    }
}

/// Tracks our forced "mute while recording" so we can restore the user's audio
/// exactly as it was. `did_mute` is true while our mute is active; `prev_muted`
/// is the system mute state captured just before we muted, used to decide
/// whether to unmute on stop (so a system that was already muted stays muted).
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct MuteState {
    pub(super) did_mute: bool,
    pub(super) prev_muted: Option<bool>,
    /// Murmur is quitting: no recording mutes any more, as a mute applied
    /// now would outlive the process.
    pub(super) quitting: bool,
}

impl MuteState {
    /// Mutes for a recording that is still `current`. Callers decide
    /// `current` under the mute lock, so a cancel or stop that invalidated the
    /// recording and then released the mute cannot be followed by a late mute.
    pub(super) fn engage(
        &mut self,
        current: bool,
        read_system: impl FnOnce() -> Option<bool>,
        mute_system: impl FnOnce(),
    ) {
        // A duplicate apply must not re-snapshot our own forced mute as the
        // user's state, or release would leave audio muted.
        if !current || self.did_mute || self.quitting {
            return;
        }
        self.prev_muted = read_system();
        mute_system();
        self.did_mute = true;
        debug!("Mute applied (prev_muted={:?})", self.prev_muted);
    }

    pub(super) fn release(&mut self, restore_system: impl FnOnce(Option<bool>)) {
        if self.did_mute {
            restore_system(self.prev_muted);
            self.did_mute = false;
            debug!("Mute removed (restored prev_muted={:?})", self.prev_muted);
        }
    }

    pub(super) fn release_for_good(&mut self, restore_system: impl FnOnce(Option<bool>)) {
        self.release(restore_system);
        self.quitting = true;
    }
}

impl AudioRecordingManager {
    /// Applies mute if mute_while_recording is enabled, the stream is open and
    /// the recording that became ready at `readiness_generation` is still
    /// current. Snapshots the system's prior mute state first so
    /// `remove_mute` can restore it instead of unconditionally unmuting.
    pub fn apply_mute(&self, readiness_generation: u64) {
        let settings = get_settings(&self.app_handle);
        if !settings.mute_while_recording {
            return;
        }

        // Lock order: is_open before mute_state (matches stop_microphone_stream).
        let is_open = self.is_open.lock().unwrap();
        let mut mute_guard = self.mute_state.lock().unwrap();
        let current = *is_open && self.is_recording_readiness_current(readiness_generation);
        mute_guard.engage(current, get_mute, || set_mute(true));
    }

    /// Removes mute if it was applied, restoring the system's prior mute state
    /// (a system already muted before recording stays muted).
    pub fn remove_mute(&self) {
        self.mute_state.lock().unwrap().release(restore_mute);
    }

    /// Removes mute as Murmur quits, and refuses it to every later recording:
    /// one may still start, and become ready, before the process ends.
    pub fn remove_mute_for_good(&self) {
        self.mute_state
            .lock()
            .unwrap()
            .release_for_good(restore_mute);
    }
}
