//! The audio of a recording on its way to the stream. The recorder's side
//! pushes each frame without ever waiting; the session's side takes them.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use tokio::sync::mpsc;

use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;

/// A minute of samples is about 4 MB. A stream that far behind is not worth
/// keeping, and the queue must not grow with a 15-minute recording.
pub(super) const MAX_WAITING: usize = 60 * TARGET_SAMPLE_RATE as usize;

pub(super) fn channel() -> (Feed, Audio) {
    let (frames_tx, frames_rx) = mpsc::unbounded_channel();
    let meter = Arc::new(Meter::default());
    let feed = Feed {
        frames: Some(frames_tx),
        meter: Arc::clone(&meter),
    };
    let audio = Audio {
        frames: frames_rx,
        meter,
    };
    (feed, audio)
}

/// The recorder's side. Dropping it is the end of the audio.
pub(super) struct Feed {
    /// `None` once nothing listens, or once too much waits.
    frames: Option<mpsc::UnboundedSender<Vec<f32>>>,
    meter: Arc<Meter>,
}

impl Feed {
    pub(super) fn push(&mut self, frame: &[f32]) {
        let Some(frames) = &self.frames else { return };
        if self.meter.waiting() + frame.len() > MAX_WAITING {
            self.meter.overflowed.store(true, Ordering::Release);
            self.frames = None;
            return;
        }
        self.meter.pushed.fetch_add(frame.len(), Ordering::AcqRel);
        if frames.send(frame.to_vec()).is_err() {
            self.frames = None;
        }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        let _ = self.meter.unsent_at_stop.set(self.meter.waiting());
    }
}

/// The session's side.
pub(super) struct Audio {
    pub(super) frames: mpsc::UnboundedReceiver<Vec<f32>>,
    pub(super) meter: Arc<Meter>,
}

/// How far the session is behind the recorder, in samples.
#[derive(Default)]
pub(super) struct Meter {
    pushed: AtomicUsize,
    sent: AtomicUsize,
    overflowed: AtomicBool,
    unsent_at_stop: OnceLock<usize>,
}

impl Meter {
    fn waiting(&self) -> usize {
        self.pushed
            .load(Ordering::Acquire)
            .saturating_sub(self.sent.load(Ordering::Acquire))
    }

    /// The session wrote this many samples to its socket.
    pub(super) fn sent(&self, samples: usize) {
        self.sent.fetch_add(samples, Ordering::AcqRel);
    }

    /// More than `MAX_WAITING` waited: the recorder's side gave up.
    pub(super) fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }

    /// The samples not yet sent when the recording stopped. `None` while it
    /// records.
    pub(super) fn unsent_at_stop(&self) -> Option<usize> {
        self.unsent_at_stop.get().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: usize = 480;

    #[test]
    fn the_session_takes_the_frames_in_the_order_they_were_pushed() {
        let (mut feed, mut audio) = channel();
        feed.push(&[0.1; FRAME]);
        feed.push(&[0.2; 7]);
        drop(feed);

        assert_eq!(audio.frames.blocking_recv(), Some(vec![0.1; FRAME]));
        assert_eq!(audio.frames.blocking_recv(), Some(vec![0.2; 7]));
        assert_eq!(audio.frames.blocking_recv(), None, "the audio ended");
    }

    #[test]
    fn what_was_not_sent_is_counted_when_the_recording_stops() {
        let (mut feed, audio) = channel();
        feed.push(&[0.1; FRAME]);
        feed.push(&[0.1; FRAME]);
        audio.meter.sent(FRAME);
        assert_eq!(audio.meter.unsent_at_stop(), None, "still recording");

        drop(feed);
        assert_eq!(audio.meter.unsent_at_stop(), Some(FRAME));
    }

    #[test]
    fn a_feed_nothing_listens_to_stops_copying_frames() {
        let (mut feed, audio) = channel();
        let meter = Arc::clone(&audio.meter);
        drop(audio);

        feed.push(&[0.1; FRAME]);
        assert!(feed.frames.is_none());
        feed.push(&[0.1; FRAME]);
        assert_eq!(meter.waiting(), FRAME, "the later frames are not counted");
    }

    #[test]
    fn more_than_a_minute_waiting_ends_the_feed() {
        let (mut feed, mut audio) = channel();
        for _ in 0..MAX_WAITING / FRAME {
            feed.push(&[0.1; FRAME]);
        }
        assert!(!audio.meter.overflowed(), "a minute still fits");

        feed.push(&[0.1; FRAME]);
        assert!(audio.meter.overflowed());
        let mut queued = 0;
        while let Some(frame) = audio.frames.blocking_recv() {
            queued += frame.len();
        }
        assert_eq!(
            queued, MAX_WAITING,
            "the queue holds a minute and is closed"
        );
    }

    #[test]
    fn audio_the_session_sent_no_longer_waits() {
        let (mut feed, audio) = channel();
        for _ in 0..MAX_WAITING / FRAME {
            feed.push(&[0.1; FRAME]);
            audio.meter.sent(FRAME);
        }
        feed.push(&[0.1; FRAME]);
        assert!(!audio.meter.overflowed());
    }
}
