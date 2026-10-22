//! When the automatic update checks run: at launch, every
//! [`CHECK_INTERVAL`], and after the Mac wakes from sleep.

use std::time::{Duration, Instant};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);
/// The wait after a failed check, doubled after each failure in a row up to
/// [`CHECK_INTERVAL`]. A check on wake can run before the network is back.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
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
    /// Checks that failed in a row, the last one included.
    failures: u32,
}

impl Schedule {
    /// Called every [`TICK`]. Whether an automatic check is due now.
    pub fn due(&mut self, now: Moment, enabled: bool) -> bool {
        let woke = self.last_tick.is_some_and(|tick| now.slept_since(tick));
        self.last_tick = Some(now);
        if !enabled {
            // Turning the checks back on checks at once, as a launch does.
            self.last_check = None;
            self.failures = 0;
            return false;
        }
        let wait = self.wait();
        let due = woke || self.last_check.is_none_or(|check| now.since(check) >= wait);
        if due {
            self.last_check = Some(now);
        }
        due
    }

    /// How the check [`Schedule::due`] asked for went.
    pub fn checked(&mut self, succeeded: bool) {
        self.failures = if succeeded { 0 } else { self.failures + 1 };
    }

    fn wait(&self) -> Duration {
        match self.failures {
            0 => CHECK_INTERVAL,
            failures => RETRY_AFTER
                .saturating_mul(1 << (failures - 1).min(16))
                .min(CHECK_INTERVAL),
        }
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

    /// Ticks for `duration` of awake time, every check that comes due ending
    /// as `succeeded`; the time between one check and the next.
    fn gaps_between_checks(
        schedule: &mut Schedule,
        clock: &mut Clock,
        duration: Duration,
        succeeded: bool,
    ) -> Vec<Duration> {
        let mut gaps = Vec::new();
        let mut since_check = Duration::ZERO;
        for _ in 0..duration.as_secs() / TICK.as_secs() {
            since_check += TICK;
            if schedule.due(clock.run(TICK), true) {
                schedule.checked(succeeded);
                gaps.push(since_check);
                since_check = Duration::ZERO;
            }
        }
        gaps
    }

    fn minutes<const N: usize>(minutes: [u64; N]) -> [Duration; N] {
        minutes.map(|minutes| Duration::from_secs(minutes * 60))
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
    fn a_failed_check_on_wake_retries_within_minutes() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();
        assert!(schedule.due(clock.now, true));
        schedule.checked(true);

        clock.sleep(8 * HOUR);
        assert!(schedule.due(clock.run(TICK), true), "on wake");
        schedule.checked(false);

        assert_eq!(
            gaps_between_checks(&mut schedule, &mut clock, HOUR, true),
            minutes([5]),
            "the network is back five minutes later"
        );
    }

    #[test]
    fn failed_checks_back_off_to_four_hours() {
        let mut clock = Clock::new();
        let mut schedule = Schedule::default();
        assert!(schedule.due(clock.now, true), "launch, offline");
        schedule.checked(false);

        assert_eq!(
            gaps_between_checks(&mut schedule, &mut clock, 24 * HOUR, false)[..8],
            minutes([5, 10, 20, 40, 80, 160, 240, 240])
        );
        assert_eq!(
            gaps_between_checks(&mut schedule, &mut clock, 12 * HOUR, true)[1..],
            [CHECK_INTERVAL, CHECK_INTERVAL],
            "back online"
        );
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
