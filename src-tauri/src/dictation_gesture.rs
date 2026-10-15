//! Reads the one recording shortcut as a gesture. Holding it is push-to-talk;
//! a quick double tap keeps recording hands-free until the next press.
//!
//! Recording starts on the first press, before the gesture is known, so the
//! first words are never cut off. A lone short tap is then discarded without
//! an upload.

use std::time::{Duration, Instant};

/// A press held at least this long is push-to-talk: its release transcribes.
pub const HOLD_THRESHOLD: Duration = Duration::from_millis(250);
/// After a short tap, a second press sooner than this locks hands-free.
pub const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Idle,
    /// The key is down and a recording was requested at `since`.
    Pressed {
        since: Instant,
    },
    /// A short tap was released; the recording keeps running until either a
    /// second press locks it or `deadline` passes.
    AwaitingSecondTap {
        deadline: Instant,
    },
    /// Recording continues without the key. `key_down` is true while the
    /// press that locked it is still held, so its release is ignored.
    HandsFree {
        key_down: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Press,
    Release,
}

/// What the coordinator must do with the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    None,
    Start,
    /// Start a recording that is already hands-free (remote toggle).
    StartHandsFree,
    LockHandsFree,
    /// Stop and transcribe.
    Stop,
    /// Drop the recording without transcribing it.
    Discard,
    /// Drop the tap's recording and start a new one: the press came too late
    /// to be a second tap, so it begins a gesture of its own.
    Restart,
}

impl Gesture {
    pub fn on_key(self, key: Key, at: Instant) -> (Gesture, Intent) {
        match (self, key) {
            (Gesture::Idle, Key::Press) => (Gesture::Pressed { since: at }, Intent::Start),
            (Gesture::Pressed { since }, Key::Release) => {
                if at.saturating_duration_since(since) >= HOLD_THRESHOLD {
                    (Gesture::Idle, Intent::Stop)
                } else {
                    (
                        Gesture::AwaitingSecondTap {
                            deadline: at + DOUBLE_TAP_WINDOW,
                        },
                        Intent::None,
                    )
                }
            }
            (Gesture::AwaitingSecondTap { deadline }, Key::Press) => {
                if at < deadline {
                    (Gesture::HandsFree { key_down: true }, Intent::LockHandsFree)
                } else {
                    (Gesture::Pressed { since: at }, Intent::Restart)
                }
            }
            (Gesture::HandsFree { key_down: true }, Key::Release) => {
                (Gesture::HandsFree { key_down: false }, Intent::None)
            }
            (Gesture::HandsFree { key_down: false }, Key::Press) => (Gesture::Idle, Intent::Stop),
            // Key repeats and the release of a press that already stopped.
            (state, _) => (state, Intent::None),
        }
    }

    pub fn deadline(self) -> Option<Instant> {
        match self {
            Gesture::AwaitingSecondTap { deadline } => Some(deadline),
            _ => None,
        }
    }

    pub fn on_deadline(self, now: Instant) -> (Gesture, Intent) {
        match self {
            Gesture::AwaitingSecondTap { deadline } if now >= deadline => {
                (Gesture::Idle, Intent::Discard)
            }
            state => (state, Intent::None),
        }
    }

    /// A remote toggle (CLI flag or SIGUSR2) has no key to hold, so the
    /// recording it starts is hands-free from the outset.
    pub fn on_toggle(self) -> (Gesture, Intent) {
        match self {
            Gesture::Idle => (
                Gesture::HandsFree { key_down: false },
                Intent::StartHandsFree,
            ),
            _ => (Gesture::Idle, Intent::Stop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    struct Run {
        gesture: Gesture,
        t0: Instant,
        intents: Vec<Intent>,
    }

    impl Run {
        fn new() -> Self {
            Self {
                gesture: Gesture::Idle,
                t0: Instant::now(),
                intents: Vec::new(),
            }
        }

        fn key(&mut self, key: Key, at_ms: u64) -> &mut Self {
            let (gesture, intent) = self.gesture.on_key(key, self.t0 + ms(at_ms));
            self.apply(gesture, intent)
        }

        fn press(&mut self, at_ms: u64) -> &mut Self {
            self.key(Key::Press, at_ms)
        }

        fn release(&mut self, at_ms: u64) -> &mut Self {
            self.key(Key::Release, at_ms)
        }

        fn tick(&mut self, at_ms: u64) -> &mut Self {
            let (gesture, intent) = self.gesture.on_deadline(self.t0 + ms(at_ms));
            self.apply(gesture, intent)
        }

        fn toggle(&mut self) -> &mut Self {
            let (gesture, intent) = self.gesture.on_toggle();
            self.apply(gesture, intent)
        }

        fn apply(&mut self, gesture: Gesture, intent: Intent) -> &mut Self {
            self.gesture = gesture;
            if intent != Intent::None {
                self.intents.push(intent);
            }
            self
        }
    }

    #[test]
    fn hold_records_while_held_and_stops_on_release() {
        let mut run = Run::new();
        run.press(0).release(1_200);
        assert_eq!(run.intents, [Intent::Start, Intent::Stop]);
        assert_eq!(run.gesture, Gesture::Idle);
    }

    #[test]
    fn release_exactly_at_the_hold_threshold_is_push_to_talk() {
        let mut run = Run::new();
        run.press(0).release(250);
        assert_eq!(run.intents, [Intent::Start, Intent::Stop]);
    }

    #[test]
    fn single_short_tap_is_discarded_once_the_double_tap_window_passes() {
        let mut run = Run::new();
        run.press(0).release(249).tick(598);
        assert_eq!(run.intents, [Intent::Start], "still waiting at 598 ms");
        run.tick(599);
        assert_eq!(run.intents, [Intent::Start, Intent::Discard]);
        assert_eq!(run.gesture, Gesture::Idle);
    }

    #[test]
    fn double_tap_locks_hands_free_until_the_next_press() {
        let mut run = Run::new();
        run.press(0).release(80).press(200).release(260);
        assert_eq!(run.intents, [Intent::Start, Intent::LockHandsFree]);
        assert_eq!(run.gesture, Gesture::HandsFree { key_down: false });

        run.tick(60_000);
        assert_eq!(run.gesture, Gesture::HandsFree { key_down: false });

        run.press(90_000).release(90_100);
        assert_eq!(
            run.intents,
            [Intent::Start, Intent::LockHandsFree, Intent::Stop]
        );
        assert_eq!(run.gesture, Gesture::Idle);
    }

    #[test]
    fn second_press_on_the_window_edge_discards_the_tap_and_starts_afresh() {
        let mut run = Run::new();
        run.press(0).release(100).press(450);
        assert_eq!(run.intents, [Intent::Start, Intent::Restart]);
        run.release(1_500);
        assert_eq!(run.intents, [Intent::Start, Intent::Restart, Intent::Stop]);
        assert_eq!(run.gesture, Gesture::Idle);
    }

    #[test]
    fn tap_then_tap_and_hold_stays_hands_free_after_the_release() {
        let mut run = Run::new();
        run.press(0).release(100).press(300).release(3_000);
        assert_eq!(run.intents, [Intent::Start, Intent::LockHandsFree]);
        assert_eq!(run.gesture, Gesture::HandsFree { key_down: false });
    }

    #[test]
    fn key_repeat_presses_while_held_are_ignored() {
        let mut run = Run::new();
        run.press(0).press(30).press(60).release(900);
        assert_eq!(run.intents, [Intent::Start, Intent::Stop]);
    }

    #[test]
    fn release_of_the_stopping_press_is_ignored() {
        let mut run = Run::new();
        run.press(0).release(50).press(100).release(150);
        run.press(5_000).release(5_400);
        assert_eq!(
            run.intents,
            [Intent::Start, Intent::LockHandsFree, Intent::Stop]
        );
        assert_eq!(run.gesture, Gesture::Idle);
    }

    #[test]
    fn stray_release_while_idle_does_nothing() {
        let mut run = Run::new();
        run.release(10);
        assert!(run.intents.is_empty());
        assert_eq!(run.gesture, Gesture::Idle);
    }

    #[test]
    fn deadline_exists_only_while_waiting_for_the_second_tap() {
        let t0 = Instant::now();
        assert_eq!(Gesture::Idle.deadline(), None);
        assert_eq!(Gesture::Pressed { since: t0 }.deadline(), None);
        assert_eq!(Gesture::HandsFree { key_down: false }.deadline(), None);
        let (waiting, _) = Gesture::Pressed { since: t0 }.on_key(Key::Release, t0 + ms(100));
        assert_eq!(waiting.deadline(), Some(t0 + ms(450)));
    }

    /// The coordinator answers the cancel shortcut by resetting the gesture.
    /// Whatever the key does next must neither stop nor discard anything, and
    /// the next real press must start a fresh recording.
    #[test]
    fn cancel_in_each_state_leaves_no_trailing_intent() {
        type Setup = (&'static str, fn(&mut Run));
        let setups: [Setup; 4] = [
            ("held", |run| {
                run.press(0);
            }),
            ("awaiting second tap", |run| {
                run.press(0).release(50);
            }),
            ("hands-free, key held", |run| {
                run.press(0).release(50).press(100);
            }),
            ("hands-free", |run| {
                run.press(0).release(50).press(100).release(150);
            }),
        ];
        for (name, setup) in setups {
            let mut run = Run::new();
            setup(&mut run);
            run.intents.clear();
            run.gesture = Gesture::Idle;

            run.release(2_000).tick(10_000);
            assert!(run.intents.is_empty(), "{name}: {:?}", run.intents);
            run.press(20_000).release(21_000);
            assert_eq!(run.intents, [Intent::Start, Intent::Stop], "{name}");
        }
    }

    #[test]
    fn remote_toggle_starts_hands_free_and_a_key_press_stops_it() {
        let mut run = Run::new();
        run.toggle().press(1_000).release(1_100);
        assert_eq!(run.intents, [Intent::StartHandsFree, Intent::Stop]);
        assert_eq!(run.gesture, Gesture::Idle);
    }

    #[test]
    fn remote_toggle_stops_any_recording_gesture() {
        for setup in [
            |run: &mut Run| {
                run.press(0);
            },
            |run: &mut Run| {
                run.press(0).release(50);
            },
            |run: &mut Run| {
                run.press(0).release(50).press(100);
            },
        ] {
            let mut run = Run::new();
            setup(&mut run);
            run.toggle();
            assert_eq!(run.intents.last(), Some(&Intent::Stop));
            assert_eq!(run.gesture, Gesture::Idle);
        }
    }
}
