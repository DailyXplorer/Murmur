//! The remote participants' track: everything the Mac plays, captured by a
//! Core Audio process tap in `swift/meeting_audio.swift` (macOS 14.4+). The
//! tap needs the "System Audio Recording" permission, not Screen Recording;
//! macOS asks for it the first time a tap starts.
//!
//! On Windows the same track comes from WASAPI loopback (`wasapi.rs`), which
//! asks for no permission.

use serde::Serialize;
use specta::Type;
use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};

use super::route::RouteFacts;
use super::{Feeder, TrackWriter};
use crate::meetings::coordinator::Track;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SystemAudioSupport {
    Available,
    /// Process taps need macOS 14.4 or later.
    #[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
    OsTooOld,
    /// This build has no system audio bridge (Intel).
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    Unsupported,
}

#[cfg(target_arch = "aarch64")]
mod bridge {
    use std::ffi::c_void;

    pub type SamplesCallback = extern "C" fn(*mut c_void, *const f32, i32, f64);

    extern "C" {
        pub fn murmur_system_audio_support() -> i32;
        pub fn murmur_system_tap_start(context: *mut c_void, on_samples: SamplesCallback) -> i32;
        /// 1 when another app played sound while the tap ran, else 0.
        pub fn murmur_system_tap_stop() -> i32;
        /// 1 while the device could not be rebuilt after an output change.
        pub fn murmur_system_tap_interrupted() -> i32;
        /// 0 allowed, 1 denied, 2 not asked yet or unknown.
        pub fn murmur_system_audio_permission() -> i32;
        /// 0 with the default output's facts written, 1 when there is none.
        pub fn murmur_output_route(
            transport: *mut u32,
            data_source: *mut u32,
            has_input: *mut i32,
        ) -> i32;
    }
}

/// The default output, as Core Audio describes it now. `None` when there is
/// none, and on Intel, which records no system audio to compare with. On
/// Windows, its form factor and bus (`None` when Windows does not say).
pub fn output_route_facts() -> Option<RouteFacts> {
    #[cfg(target_os = "windows")]
    {
        super::wasapi::output_route_facts()
    }
    #[cfg(target_arch = "aarch64")]
    {
        let (mut transport, mut data_source, mut has_input) = (0u32, 0u32, 0i32);
        // SAFETY: the three pointers are to locals that outlive the call,
        // which only writes them.
        let code = unsafe {
            bridge::murmur_output_route(&mut transport, &mut data_source, &mut has_input)
        };
        (code == 0).then_some(RouteFacts {
            transport,
            data_source: (data_source != 0).then_some(data_source),
            has_input: has_input == 1,
        })
    }
    #[cfg(not(any(target_arch = "aarch64", target_os = "windows")))]
    {
        None
    }
}

/// The "System Audio Recording" permission, as macOS reports it now.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SystemAudioPermission {
    Granted,
    Denied,
    /// Not asked yet, or macOS does not say.
    Unknown,
}

pub fn permission() -> SystemAudioPermission {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: takes no arguments and only reads the permission.
    let code = unsafe { bridge::murmur_system_audio_permission() };
    // Windows has no consent for loopback capture.
    #[cfg(target_os = "windows")]
    let code = 0;
    #[cfg(not(any(target_arch = "aarch64", target_os = "windows")))]
    let code = 2;
    match code {
        0 => SystemAudioPermission::Granted,
        1 => SystemAudioPermission::Denied,
        _ => SystemAudioPermission::Unknown,
    }
}

pub fn support() -> SystemAudioSupport {
    // Classic loopback exists since Windows Vista, and the process loopback
    // is tried first and falls back to it, so no OS build is checked.
    #[cfg(target_os = "windows")]
    {
        if cfg!(target_arch = "x86_64") {
            SystemAudioSupport::Available
        } else {
            SystemAudioSupport::Unsupported
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: takes no arguments and only reads the OS version.
        match unsafe { bridge::murmur_system_audio_support() } {
            0 => SystemAudioSupport::Available,
            _ => SystemAudioSupport::OsTooOld,
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_os = "windows")))]
    {
        SystemAudioSupport::Unsupported
    }
}

/// Mirrors the start codes in `swift/meeting_audio.swift`.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
fn start_error(code: i32) -> String {
    match code {
        1 => "System audio capture needs macOS 14.4 or later",
        2 => "System audio is already being captured",
        3 => "Cannot create the system audio tap",
        4 => "Cannot create the system audio device",
        5 => "The system audio format is not supported",
        6 => "Cannot start the system audio device",
        _ => "System audio capture failed",
    }
    .to_string()
}

/// Called by the bridge on its audio queue, never after `murmur_system_tap_stop`
/// returns.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
extern "C" fn deliver(context: *mut c_void, samples: *const f32, frames: i32, sample_rate: f64) {
    if context.is_null() || samples.is_null() || frames <= 0 {
        return;
    }
    // SAFETY: `context` is the `Feeder` boxed in `start`, alive until after
    // the tap stops; `samples` holds `frames` mono floats for this call.
    let (feeder, samples) = unsafe {
        (
            &*context.cast::<Feeder>(),
            std::slice::from_raw_parts(samples, frames as usize),
        )
    };
    feeder.push(samples.to_vec(), sample_rate.round() as u32);
}

pub struct SystemCapture {
    writer: TrackWriter,
    /// The boxed `Feeder` the bridge calls back with, as an address so the
    /// capture can move to another thread.
    #[cfg(not(target_os = "windows"))]
    context: usize,
    #[cfg(target_os = "windows")]
    capture: super::wasapi::LoopbackCapture,
}

impl SystemCapture {
    #[cfg(target_os = "windows")]
    pub fn start(path: &Path, origin: Instant) -> Result<Self, String> {
        let writer = TrackWriter::create(path, origin)?;
        let capture = super::wasapi::LoopbackCapture::start(writer.feeder())?;
        Ok(Self { writer, capture })
    }

    #[cfg(not(target_os = "windows"))]
    pub fn start(path: &Path, origin: Instant) -> Result<Self, String> {
        let writer = TrackWriter::create(path, origin)?;
        let context = Box::into_raw(Box::new(writer.feeder())).cast::<c_void>();
        #[cfg(target_arch = "aarch64")]
        // SAFETY: `context` stays valid until `finish` stops the tap.
        let code = unsafe { bridge::murmur_system_tap_start(context, deliver) };
        #[cfg(not(target_arch = "aarch64"))]
        let code = -1;
        if code != 0 {
            // SAFETY: the bridge did not start, so it holds no reference.
            drop(unsafe { Box::from_raw(context.cast::<Feeder>()) });
            return Err(start_error(code));
        }
        Ok(Self {
            writer,
            context: context as usize,
        })
    }
}

impl Track for SystemCapture {
    fn failed(&self) -> bool {
        self.writer.failed()
    }

    fn interrupted(&self) -> bool {
        #[cfg(target_os = "windows")]
        return self.capture.interrupted();
        #[cfg(target_arch = "aarch64")]
        // SAFETY: takes no arguments and only reads the tap's state.
        return unsafe { bridge::murmur_system_tap_interrupted() } == 1;
        #[cfg(not(any(target_arch = "aarch64", target_os = "windows")))]
        false
    }

    fn dropped_ms(&self) -> u64 {
        self.writer.dropped_ms()
    }

    fn live(&self, tap: crate::meetings::live::LiveTap) {
        self.writer.live(tap);
    }

    /// On Windows the answer is always "no": it only feeds the check for a
    /// denied System Audio Recording permission (zeros while another app
    /// played), which Windows has no counterpart for. A silent loopback there
    /// means nothing played.
    #[cfg(target_os = "windows")]
    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        let this = *self;
        // Stops the stream and joins its thread: no packet comes after.
        drop(this.capture);
        this.writer.finish(duration).map(|_| false)
    }

    #[cfg(not(target_os = "windows"))]
    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: stops the tap started in `start`; no callback runs after it.
        let played = unsafe { bridge::murmur_system_tap_stop() } == 1;
        #[cfg(not(target_arch = "aarch64"))]
        let played = false;
        // SAFETY: the tap is stopped, so nothing else uses the feeder.
        drop(unsafe { Box::from_raw(self.context as *mut Feeder) });
        self.writer.finish(duration).map(|_| played)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_with_a_backend_answer_through_it() {
        assert_eq!(
            support() == SystemAudioSupport::Unsupported,
            cfg!(not(any(
                target_arch = "aarch64",
                all(target_os = "windows", target_arch = "x86_64")
            )))
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_asks_for_no_permission() {
        assert_eq!(permission(), SystemAudioPermission::Granted);
    }

    /// A strong link to an API added in macOS 14.2 would stop the app from
    /// launching on older macOS.
    #[test]
    #[cfg(target_arch = "aarch64")]
    fn the_tap_api_is_weak_linked() {
        let executable = std::env::current_exe().unwrap();
        let output = std::process::Command::new("nm")
            .arg("-m")
            .arg(&executable)
            .output()
            .unwrap();
        let symbols = String::from_utf8_lossy(&output.stdout);
        for symbol in [
            "_AudioHardwareCreateProcessTap",
            "_AudioHardwareDestroyProcessTap",
            "_OBJC_CLASS_$_CATapDescription",
        ] {
            let line = symbols
                .lines()
                .find(|line| line.ends_with(&format!("{symbol} (from CoreAudio)")))
                .unwrap_or_else(|| panic!("{symbol} is not linked"));
            assert!(line.contains("weak external"), "{line}");
        }
    }

    #[test]
    fn every_bridge_start_code_has_its_own_message() {
        let messages: std::collections::HashSet<_> = (1..=6).map(start_error).collect();
        assert_eq!(messages.len(), 6);
    }

    #[test]
    fn support_serializes_as_the_frontend_reads_it() {
        assert_eq!(
            serde_json::to_value(SystemAudioSupport::OsTooOld).unwrap(),
            "os_too_old"
        );
    }
}
