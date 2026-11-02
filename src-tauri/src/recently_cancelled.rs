//! The last cancelled dictation, kept in memory for a minute so a cancel the
//! user did not mean costs a click on Restore, not the dictation. Nothing
//! here is written to disk. The audio is zeroed when the minute is over, when
//! a new recording starts, when another cancel replaces it, and on quit.

use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::managers::audio::AudioRecordingManager;
use crate::managers::transcription::Transcript;
use crate::overlay::DictationEpoch;
use crate::TranscriptionCoordinator;
use log::debug;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use zeroize::Zeroize;

pub const KEEP_FOR: Duration = Duration::from_secs(60);
/// A cancel with less audio than this, one second, keeps nothing.
pub(crate) const MIN_SAMPLES: usize = TARGET_SAMPLE_RATE as usize;

/// A cancelled dictation, until Restore takes it or it expires.
pub struct Kept {
    pub samples: Vec<f32>,
    /// Its transcript, when the engine had answered before the cancel.
    pub transcript: Option<Transcript>,
}

impl Kept {
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / TARGET_SAMPLE_RATE as f64)
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.samples.zeroize();
        if let Some(transcript) = &mut self.transcript {
            transcript.text.zeroize();
        }
    }
}

#[derive(Default)]
struct Slot {
    kept: Option<Kept>,
    /// Bumped by every keep, so an expiry only drops the dictation it was
    /// started for.
    generation: u64,
    /// Set when the app quits. Checked and set under the slot's lock, like
    /// the keep itself, so no pipeline that unwinds later can refill it.
    closed: bool,
}

/// One slot: a second cancel replaces the first.
#[derive(Default)]
pub struct RecentlyCancelled(Mutex<Slot>);

impl RecentlyCancelled {
    fn slot(&self) -> MutexGuard<'_, Slot> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keeps `kept` in place of any earlier one and returns the generation
    /// its expiry needs, or None when it is too short to be worth a Restore
    /// or the app is quitting.
    fn keep(&self, kept: Kept) -> Option<u64> {
        if kept.samples.len() < MIN_SAMPLES {
            return None;
        }
        let mut slot = self.slot();
        if slot.closed {
            return None;
        }
        slot.generation = slot.generation.wrapping_add(1);
        slot.kept = Some(kept);
        Some(slot.generation)
    }

    pub fn take(&self) -> Option<Kept> {
        self.slot().kept.take()
    }

    /// The kept audio's length, for the tray item.
    pub fn duration(&self) -> Option<Duration> {
        self.slot().kept.as_ref().map(Kept::duration)
    }

    fn clear(&self) {
        // Dropped outside the lock: zeroing a long dictation takes a moment.
        let kept = self.slot().kept.take();
        drop(kept);
    }

    /// As `clear`, and nothing is kept from now on.
    fn close(&self) {
        let kept = {
            let mut slot = self.slot();
            slot.closed = true;
            slot.kept.take()
        };
        drop(kept);
    }

    /// Drops the dictation kept as `generation` once `KEEP_FOR` has passed.
    /// Returns whether it was still there.
    async fn expire_after(&self, generation: u64) -> bool {
        tokio::time::sleep(KEEP_FOR).await;
        let kept = {
            let mut slot = self.slot();
            if slot.generation != generation {
                return false;
            }
            slot.kept.take()
        };
        kept.is_some()
    }
}

/// Keeps a cancelled dictation for `KEEP_FOR` and offers Restore: a pill for
/// a few seconds, and a tray item for the whole minute. `shown` is the overlay
/// epoch of the cancelled dictation, so its pill never covers a newer one.
pub fn keep(
    app: &AppHandle,
    samples: Vec<f32>,
    transcript: Option<Transcript>,
    shown: DictationEpoch,
) {
    let kept = Kept {
        samples,
        transcript,
    };
    // A recording that started since owns the overlay.
    let recording = app
        .try_state::<Arc<AudioRecordingManager>>()
        .is_some_and(|audio| audio.is_recording());
    if recording {
        return;
    }
    let Some(slot) = app.try_state::<Arc<RecentlyCancelled>>() else {
        return;
    };
    let duration = kept.duration();
    let Some(generation) = slot.keep(kept) else {
        debug!("Cancelled dictation not kept: too short, or the app is quitting");
        return;
    };
    debug!(
        "Kept a cancelled dictation of {:.1} s for {} s",
        duration.as_secs_f64(),
        KEEP_FOR.as_secs()
    );
    let slot = Arc::clone(&slot);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if slot.expire_after(generation).await {
            debug!("Cancelled dictation expired and was wiped");
            crate::tray::update_tray_menu(&handle, None);
        }
    });
    crate::tray::update_tray_menu(app, None);
    crate::overlay::show_cancelled_notice(app, shown);
}

/// Zeroes and drops the kept dictation, if any.
pub fn wipe(app: &AppHandle) {
    if let Some(slot) = app.try_state::<Arc<RecentlyCancelled>>() {
        slot.clear();
    }
}

/// Zeroes the kept dictation and keeps none from now on. Both ways of
/// quitting call it: a pipeline the quit cancels still unwinds and tries to
/// keep its audio, whenever that happens relative to the quit's own steps.
pub fn close(app: &AppHandle) {
    if let Some(slot) = app.try_state::<Arc<RecentlyCancelled>>() {
        slot.close();
    }
}

/// The kept dictation's tray label duration, as `m:ss`.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Restores the kept dictation, from the pill or the tray. The coordinator
/// runs it only while no dictation records or transcribes.
pub fn request_restore(app: &AppHandle) {
    if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
        coordinator.send_restore_cancelled();
    }
}

/// The cancelled pill's Restore button.
#[tauri::command]
#[specta::specta]
pub fn restore_cancelled_dictation(app: AppHandle) {
    request_restore(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds_of_audio(seconds: f64) -> Kept {
        Kept {
            samples: vec![0.25; (seconds * TARGET_SAMPLE_RATE as f64) as usize],
            transcript: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn kept_audio_expires_after_60s() {
        let slot = Arc::new(RecentlyCancelled::default());
        let generation = slot.keep(seconds_of_audio(30.0)).expect("kept");
        let expiry = tokio::spawn({
            let slot = Arc::clone(&slot);
            async move { slot.expire_after(generation).await }
        });

        tokio::time::sleep(KEEP_FOR - Duration::from_millis(1)).await;
        assert_eq!(slot.duration(), Some(Duration::from_secs(30)));

        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(expiry.await.unwrap(), "the expiry dropped the dictation");
        assert_eq!(slot.duration(), None);
        assert!(slot.take().is_none(), "nothing is left to restore");
    }

    #[test]
    fn short_cancel_keeps_nothing() {
        let slot = RecentlyCancelled::default();
        assert_eq!(slot.keep(seconds_of_audio(0.9)), None);
        assert!(slot.take().is_none());
        assert!(slot.keep(seconds_of_audio(1.0)).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn second_cancel_replaces_first() {
        let slot = Arc::new(RecentlyCancelled::default());
        let first = slot.keep(seconds_of_audio(10.0)).unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        let second = slot.keep(seconds_of_audio(20.0)).unwrap();

        // The first cancel's expiry leaves the second alone.
        assert!(!slot.expire_after(first).await);
        assert_eq!(slot.duration(), Some(Duration::from_secs(20)));
        assert!(slot.expire_after(second).await);
        assert_eq!(slot.duration(), None);
    }

    #[test]
    fn take_empties_the_slot() {
        let slot = RecentlyCancelled::default();
        slot.keep(seconds_of_audio(2.0)).unwrap();
        let kept = slot.take().expect("restorable");
        assert_eq!(kept.duration(), Duration::from_secs(2));
        assert!(slot.take().is_none(), "a dictation restores once");
    }

    #[test]
    fn a_new_recording_wipes_the_kept_dictation() {
        let slot = RecentlyCancelled::default();
        slot.keep(seconds_of_audio(5.0)).unwrap();
        slot.clear();
        assert_eq!(slot.duration(), None);
    }

    #[test]
    fn a_cancel_that_unwinds_after_the_quit_keeps_nothing() {
        let slot = RecentlyCancelled::default();
        slot.keep(seconds_of_audio(5.0)).unwrap();

        slot.close();
        assert_eq!(slot.duration(), None, "the quit zeroed what was kept");

        // The pipeline the quit cancelled reaches its keep afterwards.
        assert_eq!(slot.keep(seconds_of_audio(5.0)), None);
        assert!(slot.take().is_none());
        slot.clear();
        assert_eq!(
            slot.keep(seconds_of_audio(5.0)),
            None,
            "clear reopens nothing"
        );
    }

    #[test]
    fn tray_duration_reads_minutes_and_seconds() {
        assert_eq!(format_duration(Duration::from_secs(42)), "0:42");
        assert_eq!(format_duration(Duration::from_millis(61_900)), "1:01");
        assert_eq!(format_duration(Duration::from_secs(900)), "15:00");
    }
}
