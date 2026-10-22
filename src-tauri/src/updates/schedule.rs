//! When the automatic update checks run: at launch, every
//! [`CHECK_INTERVAL`], and after the Mac wakes from sleep.

use std::time::{Duration, Instant};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);
/// How often the schedule reads the clocks. A wake is noticed within it.
pub const TICK: Duration = Duration::from_secs(60);
/// Between two ticks, this much more wall-clock than awake time means the
/// Mac slept in between.
const SLEEP_GAP: Duration = Duration::from_secs(60);

/// A point in time on both clocks. `Instant` stands still while the Mac
/// sleeps, the wall clock does not.
#[derive(Clone, Copy, Debug)]
pub struct Moment {
    awake: Instant,
    wall_ms: i64,
}

impl Moment {
    pub fn now() -> Self {
        Self {
            awake: Instant::now(),
            wall_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    fn awake_since(self, earlier: Moment) -> Duration {
        self.awake.saturating_duration_since(earlier.awake)
    }

    fn wall_since(self, earlier: Moment) -> Duration {
        u64::try_from(self.wall_ms - earlier.wall_ms).map_or(Duration::ZERO, Duration::from_millis)
    }

    /// Wall-clock time since `earlier`; the awake time guards against the
    /// clock being set back.
    fn since(self, earlier: Moment) -> Duration {
        self.awake_since(earlier).max(self.wall_since(earlier))
    }

    fn slept_since(self, earlier: Moment) -> bool {
        self.wall_since(earlier)
            .saturating_sub(self.awake_since(earlier))
            >= SLEEP_GAP
    }
}

#[derive(Default)]
pub struct Schedule {
    last_tick: Option<Moment>,
    last_check: Option<Moment>,
}

impl Schedule {
    /// Called every [`TICK`]. Whether an automatic check is due now.
    pub fn due(&mut self, now: Moment, enabled: bool) -> bool {
        let woke = self.last_tick.is_some_and(|tick| now.slept_since(tick));
        self.last_tick = Some(now);
        if !enabled {
            // Turning the checks back on checks at once, as a launch does.
            self.last_check = None;
            return false;
        }
        let due = woke
            || self
                .last_check
                .is_none_or(|check| now.since(check) >= CHECK_INTERVAL);
        if due {
            self.last_check = Some(now);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(60 * 60);

    /// A clock the test moves: awake time and wall time advance together,
    /// except while the Mac sleeps.
    struct Clock {
        now: Moment,
    }

    impl Clock {
        fn new() -> Self {
            Self {
                now: Moment {
                    awake: Instant::now(),
                    wall_ms: 1_760_000_000_000,
                },
            }
        }

        fn run(&mut self, duration: Duration) -> Moment {
            self.now.awake += duration;
            self.sleep(duration)
        }

        fn sleep(&mut self, duration: Duration) -> Moment {
            self.now.wall_ms += duration.as_millis() as i64;
            self.now
        }
    }

    /// Ticks for `duration` of awake time; how many checks came due.
    fn checks_while_running(
        schedule: &mut Schedule,
        clock: &mut Clock,
        duration: Duration,
        enabled: bool,
    ) -> usize {
        let ticks = duration.as_secs() / TICK.as_secs();
        (0..ticks)
            .filter(|_| schedule.due(clock.run(TICK), enabled))
            .count()
    }

    #[test]
    fn checks_at_launch_then_every_four_hours() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();

        assert!(schedule.due(clock.now, true), "launch checks");
        assert_eq!(
            checks_while_running(&mut schedule, &mut clock, 4 * HOUR - TICK, true),
            0,
            "nothing before four hours"
        );
        assert!(
            schedule.due(clock.run(TICK), true),
            "four hours after launch"
        );
        assert_eq!(
            checks_while_running(&mut schedule, &mut clock, 20 * HOUR, true),
            5,
            "one check per four hours over a long run"
        );
    }

    #[test]
    fn waking_from_sleep_checks() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();
        assert!(schedule.due(clock.now, true));
        assert!(!schedule.due(clock.run(TICK), true));

        clock.sleep(Duration::from_secs(20 * 60));
        assert!(
            schedule.due(clock.run(TICK), true),
            "first tick after waking"
        );
        assert!(!schedule.due(clock.run(TICK), true), "once per wake");
    }

    #[test]
    fn a_long_sleep_does_not_check_twice() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();
        assert!(schedule.due(clock.now, true));

        clock.sleep(10 * HOUR);
        assert!(schedule.due(clock.run(TICK), true));
        assert_eq!(
            checks_while_running(&mut schedule, &mut clock, HOUR, true),
            0
        );
    }

    #[test]
    fn turned_off_never_checks() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();

        assert!(!schedule.due(clock.now, false), "not at launch");
        clock.sleep(HOUR);
        assert!(!schedule.due(clock.run(TICK), false), "not on wake");
        assert_eq!(
            checks_while_running(&mut schedule, &mut clock, 24 * HOUR, false),
            0,
            "not on the interval"
        );
    }

    #[test]
    fn turning_checks_on_checks_at_once() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();
        assert!(schedule.due(clock.now, true));
        assert!(!schedule.due(clock.run(TICK), false));

        assert!(schedule.due(clock.run(TICK), true));
    }

    #[test]
    fn setting_the_clock_back_still_checks_on_time() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();
        assert!(schedule.due(clock.now, true));
        clock.now.wall_ms -= (24 * HOUR).as_millis() as i64;

        assert_eq!(
            checks_while_running(&mut schedule, &mut clock, 4 * HOUR, true),
            1
        );
    }
}
