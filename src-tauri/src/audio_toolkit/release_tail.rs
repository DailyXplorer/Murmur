//! How long a dictation keeps recording after the key is released. A person
//! often lets go of the key while the last word is still leaving their mouth,
//! so the capture runs a short tail: always `BASE_MS`, and while sustained
//! speech is still heard, on until it falls quiet or `CAP_MS` is reached.
//! A silent user pays only the base tail.
//!
//! The state machine is pure: it is fed the 20 ms frame levels of the audio
//! recorded after the release (`gain::frame_levels`) and says when to stop.
//! The speech threshold is the one meetings use (`chunking::speech_threshold`).

use super::constants::TARGET_SAMPLE_RATE;
use super::gain::frame_levels;
use crate::meetings::chunking::{speech_threshold, FRAME_MS};
use std::time::Duration;

/// The tail every dictation gets.
pub const BASE_MS: u64 = 200;
/// Unbroken speech this long, after the release, counts as the user still
/// talking.
pub const SPEECH_MS: u64 = 100;
/// The tail never runs past this, however long the speech goes on.
pub const CAP_MS: u64 = 600;
/// A tail extended by speech ends after this much quiet.
pub const QUIET_MS: u64 = 160;
/// The stop sound plays through the speakers just after the release and the
/// microphone hears it. Its duration plus this margin is ignored.
pub const LEAK_MARGIN_MS: u64 = 60;
/// A shorter recording is a tap, not a dictation: no tail.
pub const MIN_CLIP_MS: u64 = 250;

/// What `ReleaseTail::push_frame` tells the capture to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Continue,
    Finish,
}

/// The tail's limits, in 20 ms frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TailConfig {
    base_frames: u32,
    speech_frames: u32,
    cap_frames: u32,
    quiet_frames: u32,
    guard_frames: u32,
}

fn frames(ms: u64) -> u32 {
    ms.div_ceil(FRAME_MS) as u32
}

impl TailConfig {
    /// `min_ms` is the debug setting `extra_recording_buffer_ms`: the tail is
    /// never shorter, so a long value is the fixed buffer it always was.
    /// `leak_guard_ms` is how long the stop sound is ignored.
    pub fn new(min_ms: u64, leak_guard_ms: u64) -> Self {
        Self {
            base_frames: frames(BASE_MS.max(min_ms)),
            speech_frames: frames(SPEECH_MS),
            cap_frames: frames(CAP_MS.max(min_ms)),
            quiet_frames: frames(QUIET_MS),
            guard_frames: frames(leak_guard_ms),
        }
    }

    /// The longest the tail can last.
    pub fn cap(&self) -> Duration {
        Duration::from_millis(u64::from(self.cap_frames) * FRAME_MS)
    }
}

/// The span of the stop sound the microphone must not be believed during.
/// `None` when no sound plays.
pub fn leak_guard_ms(stop_sound: Option<Duration>) -> u64 {
    stop_sound.map_or(0, |length| length.as_millis() as u64 + LEAK_MARGIN_MS)
}

/// The level above which a tail frame is speech, from the recording so far
/// (the whole of it: pauses inside it anchor the noise floor; a 15-minute
/// clip is scanned in a few milliseconds). `None` for a tap, or a recording
/// with no signal at all.
pub fn threshold_for(recorded: &[f32]) -> Option<f32> {
    let min_samples = (MIN_CLIP_MS * u64::from(TARGET_SAMPLE_RATE) / 1_000) as usize;
    if recorded.len() < min_samples {
        return None;
    }
    speech_threshold(&frame_levels(recorded))
}

#[derive(Debug)]
pub struct ReleaseTail {
    config: TailConfig,
    threshold: f32,
    frames: u32,
    /// Consecutive speech frames up to now.
    voiced_run: u32,
    /// Consecutive frames that were not speech up to now.
    quiet_run: u32,
    /// Sustained speech was heard, so the tail waits for quiet.
    extended: bool,
}

impl ReleaseTail {
    pub fn new(config: TailConfig, threshold: f32) -> Self {
        Self {
            config,
            threshold,
            frames: 0,
            voiced_run: 0,
            quiet_run: 0,
            extended: false,
        }
    }

    /// Feeds the level in dBFS of the next 20 ms frame after the release.
    pub fn push_frame(&mut self, level: f32) -> Step {
        self.frames += 1;
        let leaked = self.frames <= self.config.guard_frames;
        if leaked || level < self.threshold {
            self.voiced_run = 0;
            self.quiet_run += 1;
        } else {
            self.voiced_run += 1;
            self.quiet_run = 0;
        }
        if self.voiced_run >= self.config.speech_frames {
            self.extended = true;
        }

        if self.frames >= self.config.cap_frames {
            return Step::Finish;
        }
        if self.frames < self.config.base_frames {
            return Step::Continue;
        }
        let waiting = if self.extended {
            self.quiet_run < self.config.quiet_frames
        } else {
            // A word that began as the base ran out is not cut before it is
            // known whether it is one.
            self.voiced_run > 0
        };
        if waiting {
            Step::Continue
        } else {
            Step::Finish
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEECH: f32 = -25.0;
    const ROOM: f32 = -60.0;
    const THRESHOLD: f32 = -50.0;

    fn config() -> TailConfig {
        TailConfig::new(0, 0)
    }

    /// The tail's length in ms for a level sequence, `ROOM` after its end.
    fn tail_ms(config: TailConfig, levels: &[f32]) -> u64 {
        let mut tail = ReleaseTail::new(config, THRESHOLD);
        for index in 0.. {
            let level = levels.get(index).copied().unwrap_or(ROOM);
            if tail.push_frame(level) == Step::Finish {
                return (index as u64 + 1) * FRAME_MS;
            }
        }
        unreachable!()
    }

    fn speech_for(ms: u64) -> Vec<f32> {
        vec![SPEECH; (ms / FRAME_MS) as usize]
    }

    #[test]
    fn a_silent_user_gets_only_the_base_tail() {
        assert_eq!(tail_ms(config(), &[]), BASE_MS);
    }

    #[test]
    fn speech_that_stopped_before_the_base_ran_out_adds_nothing() {
        // 80 ms is a syllable, not sustained speech.
        assert_eq!(tail_ms(config(), &speech_for(80)), BASE_MS);
    }

    #[test]
    fn sustained_speech_extends_the_tail_until_quiet() {
        // Speech through 300 ms, then 160 ms of quiet.
        assert_eq!(tail_ms(config(), &speech_for(300)), 300 + QUIET_MS);
    }

    #[test]
    fn speech_that_begins_late_in_the_base_is_still_followed() {
        let mut levels = vec![ROOM; 7];
        levels.extend(speech_for(200));
        assert_eq!(tail_ms(config(), &levels), 140 + 200 + QUIET_MS);
    }

    #[test]
    fn a_word_that_starts_as_the_base_ends_is_not_cut() {
        // Speech from 180 ms on: only 20 ms of it fits in the base, and the
        // tail waits to find out whether it is a word.
        let mut word = vec![ROOM; 9];
        word.extend(speech_for(100));
        assert_eq!(tail_ms(config(), &word), 280 + QUIET_MS);
        // A syllable that is over before it can be one ends the wait.
        let mut syllable = vec![ROOM; 9];
        syllable.extend(speech_for(60));
        assert_eq!(tail_ms(config(), &syllable), 260);
        // So does a click.
        let mut click = vec![ROOM; 9];
        click.push(SPEECH);
        assert_eq!(tail_ms(config(), &click), BASE_MS + FRAME_MS);
    }

    #[test]
    fn continuous_speech_ends_at_the_cap() {
        assert_eq!(tail_ms(config(), &speech_for(5_000)), CAP_MS);
    }

    #[test]
    fn a_short_dip_inside_speech_does_not_end_the_tail() {
        let mut levels = speech_for(200);
        levels.extend([ROOM; 3]);
        levels.extend(speech_for(100));
        assert_eq!(tail_ms(config(), &levels), 200 + 60 + 100 + QUIET_MS);
    }

    #[test]
    fn the_stop_sound_is_not_mistaken_for_speech() {
        let guard = leak_guard_ms(Some(Duration::from_millis(40)));
        assert_eq!(guard, 100);
        let config = TailConfig::new(0, guard);
        // A loud tone through the guard, then room: only the base tail.
        assert_eq!(tail_ms(config, &speech_for(100)), BASE_MS);
        // Speech that goes on past the guard is believed.
        assert_eq!(tail_ms(config, &speech_for(300)), 300 + QUIET_MS);
    }

    #[test]
    fn a_stop_sound_longer_than_the_base_leaves_only_the_base_tail() {
        // Nothing heard inside the guard counts, and the base tail is over
        // before a guard this long is.
        let guard = leak_guard_ms(Some(Duration::from_millis(790)));
        assert_eq!(
            tail_ms(TailConfig::new(0, guard), &speech_for(5_000)),
            BASE_MS
        );
        assert_eq!(tail_ms(TailConfig::new(0, guard), &[]), BASE_MS);
    }

    #[test]
    fn no_sound_means_nothing_is_ignored() {
        assert_eq!(leak_guard_ms(None), 0);
    }

    #[test]
    fn the_debug_buffer_is_a_minimum() {
        let config = TailConfig::new(1_000, 0);
        assert_eq!(tail_ms(config, &[]), 1_000);
        assert_eq!(tail_ms(config, &speech_for(5_000)), 1_000);
        // Below the base it changes nothing.
        assert_eq!(tail_ms(TailConfig::new(100, 0), &[]), BASE_MS);
        // Between base and cap it lifts the quiet floor only.
        assert_eq!(tail_ms(TailConfig::new(400, 0), &[]), 400);
        assert_eq!(config.cap(), Duration::from_millis(1_000));
    }

    #[test]
    fn a_tap_gets_no_tail_and_a_silent_clip_neither() {
        let rate = TARGET_SAMPLE_RATE as usize;
        let tap = vec![0.1; rate / 8];
        assert_eq!(threshold_for(&tap), None);
        assert_eq!(threshold_for(&vec![0.0; rate]), None);
        assert!(threshold_for(&vec![0.1; rate]).is_some());
    }
}
