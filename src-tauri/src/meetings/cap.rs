//! The 3-hour meeting limit: a warning 5 minutes before, then an automatic
//! stop. And the Mac sleeping during a meeting, which records nothing.

use std::time::Duration;

pub const CAP: Duration = Duration::from_secs(3 * 60 * 60);
pub const WARN_BEFORE: Duration = Duration::from_secs(5 * 60);

/// Wall-clock time past which a gap between two ticks is a sleep. Less is
/// the clock being corrected.
pub const SLEEP_GAP: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapAction {
    None,
    Warn,
    Stop,
}

/// What the recording needs at `elapsed`; `warned` is whether the warning was
/// already given, so it is given once.
pub fn cap_action(elapsed: Duration, warned: bool) -> CapAction {
    if elapsed >= CAP {
        CapAction::Stop
    } else if !warned && elapsed >= CAP - WARN_BEFORE {
        CapAction::Warn
    } else {
        CapAction::None
    }
}

/// How long the Mac slept between two ticks that are `awake` apart on
/// `Instant`, which stands still in sleep, and `wall_ms` apart on the wall
/// clock.
pub fn slept(awake: Duration, wall_ms: i64) -> Option<Duration> {
    let wall = Duration::from_millis(u64::try_from(wall_ms).ok()?);
    wall.checked_sub(awake).filter(|gap| *gap > SLEEP_GAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WARN_AT: Duration = Duration::from_secs(2 * 3600 + 55 * 60);

    #[test]
    fn nothing_happens_before_the_warning_time() {
        assert_eq!(cap_action(Duration::ZERO, false), CapAction::None);
        assert_eq!(
            cap_action(WARN_AT - Duration::from_millis(1), false),
            CapAction::None
        );
    }

    #[test]
    fn warns_once_five_minutes_before_the_cap() {
        assert_eq!(cap_action(WARN_AT, false), CapAction::Warn);
        assert_eq!(cap_action(WARN_AT, true), CapAction::None);
        assert_eq!(
            cap_action(CAP - Duration::from_millis(1), true),
            CapAction::None
        );
    }

    #[test]
    fn stops_at_three_hours_even_without_a_warning() {
        assert_eq!(cap_action(CAP, true), CapAction::Stop);
        assert_eq!(cap_action(CAP, false), CapAction::Stop);
        assert_eq!(
            cap_action(CAP + Duration::from_secs(90), false),
            CapAction::Stop
        );
    }

    #[test]
    fn wall_clock_time_without_instant_time_is_a_sleep() {
        assert_eq!(
            slept(Duration::from_secs(1), 20 * 60_000 + 1_000),
            Some(Duration::from_secs(20 * 60))
        );
        assert_eq!(
            slept(Duration::from_secs(1), 11_001),
            Some(Duration::from_millis(10_001))
        );
    }

    #[test]
    fn a_few_seconds_of_skew_or_a_clock_set_back_is_not_a_sleep() {
        assert_eq!(slept(Duration::from_secs(1), 1_000), None);
        assert_eq!(slept(Duration::from_secs(1), 3_000), None);
        assert_eq!(slept(Duration::from_secs(1), 11_000), None);
        assert_eq!(slept(Duration::from_secs(1), -60_000), None);
        assert_eq!(slept(Duration::from_secs(30), 1_000), None);
    }
}
