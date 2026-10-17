//! The remote participants' track: everything the Mac plays, captured by a
//! Core Audio process tap in `swift/meeting_audio.swift` (macOS 14.4+). The
//! tap needs the "System Audio Recording" permission, not Screen Recording;
//! macOS asks for it the first time a tap starts.

use serde::Serialize;
use specta::Type;
use std::ffi::c_void;
use std::path::Path;
use std::time::{Duration, Instant};

use super::{Feeder, TrackWriter};
use crate::meetings::coordinator::Track;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SystemAudioSupport {
    Available,
    /// Process taps need macOS 14.4 or later.
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
        pub fn murmur_system_tap_stop();
    }
}

pub fn support() -> SystemAudioSupport {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: takes no arguments and only reads the OS version.
        match unsafe { bridge::murmur_system_audio_support() } {
            0 => SystemAudioSupport::Available,
            _ => SystemAudioSupport::OsTooOld,
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
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
    context: usize,
}

impl SystemCapture {
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

    fn finish(self: Box<Self>, duration: Duration) -> Result<(), String> {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: stops the tap started in `start`; no callback runs after it.
        unsafe {
            bridge::murmur_system_tap_stop()
        };
        // SAFETY: the tap is stopped, so nothing else uses the feeder.
        drop(unsafe { Box::from_raw(self.context as *mut Feeder) });
        self.writer.finish(duration).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_silicon_builds_answer_through_the_bridge() {
        assert_eq!(
            support() == SystemAudioSupport::Unsupported,
            cfg!(not(target_arch = "aarch64"))
        );
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
