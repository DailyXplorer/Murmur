//! The 3-hour meeting limit: a warning 5 minutes before, then an automatic stop.

use std::time::Duration;

pub const CAP: Duration = Duration::from_secs(3 * 60 * 60);
pub const WARN_BEFORE: Duration = Duration::from_secs(5 * 60);

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
}
