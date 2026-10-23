//! How long a meeting records: every 3 hours of audio it rolls into a new
//! part, which keeps each part's files in the range processing is measured
//! on, and it stops after 12 hours, warned 5 minutes before. And the Mac
//! sleeping during a meeting, which records nothing.

use std::time::Duration;

/// The audio of one part.
pub const PART: Duration = Duration::from_secs(3 * 60 * 60);
/// The whole meeting, every part included.
pub const MAX_TOTAL: Duration = Duration::from_secs(12 * 60 * 60);
pub const WARN_BEFORE: Duration = Duration::from_secs(5 * 60);

/// Wall-clock time past which a gap between two ticks is a sleep. Less is
/// the clock being corrected.
pub const SLEEP_GAP: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapAction {
    None,
    Warn,
    /// Go on recording into the next part.
    Roll,
    Stop,
}

/// What the recording needs once the meeting has run for `total` and its
/// current part holds `part` of audio, with parts of `part_length`; `warned`
/// is whether the stop was already announced, so it is announced once.
pub fn cap_action(
    total: Duration,
    part: Duration,
    part_length: Duration,
    warned: bool,
) -> CapAction {
    if total >= MAX_TOTAL {
        CapAction::Stop
    } else if part >= part_length {
        CapAction::Roll
    } else if !warned && total >= MAX_TOTAL - WARN_BEFORE {
        CapAction::Warn
    } else {
        CapAction::None
    }
}

/// The length of a part: `PART`, or in a debug build the seconds
/// `MURMUR_MEETING_CAP_SECS` sets, to try a roll without waiting 3 hours.
pub fn part_length() -> Duration {
    #[cfg(debug_assertions)]
    if let Some(seconds) = std::env::var("MURMUR_MEETING_CAP_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
    {
        return Duration::from_secs(seconds);
    }
    PART
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

    const HOUR: Duration = Duration::from_secs(3600);
    const WARN_AT: Duration = Duration::from_secs(11 * 3600 + 55 * 60);
    const MS: Duration = Duration::from_millis(1);

    fn action(total: Duration, part: Duration, warned: bool) -> CapAction {
        cap_action(total, part, PART, warned)
    }

    #[test]
    fn a_part_rolls_into_the_next_one_at_three_hours() {
        assert_eq!(action(PART - MS, PART - MS, false), CapAction::None);
        assert_eq!(action(PART, PART, false), CapAction::Roll);
        // The fourth part of a 9-hour meeting.
        assert_eq!(action(9 * HOUR, PART, false), CapAction::Roll);
        assert_eq!(action(9 * HOUR, HOUR, false), CapAction::None);
    }

    #[test]
    fn nothing_is_announced_before_twelve_hours_minus_five_minutes() {
        // 2:55 warned before the cap became a roll.
        let old_warning = Duration::from_secs(2 * 3600 + 55 * 60);
        for total in [Duration::ZERO, old_warning, WARN_AT - MS] {
            assert_eq!(action(total, HOUR, false), CapAction::None);
        }
    }

    #[test]
    fn the_stop_is_announced_once_five_minutes_before_twelve_hours() {
        assert_eq!(action(WARN_AT, HOUR, false), CapAction::Warn);
        assert_eq!(action(WARN_AT, HOUR, true), CapAction::None);
        assert_eq!(action(MAX_TOTAL - MS, HOUR, true), CapAction::None);
    }

    #[test]
    fn the_meeting_stops_at_twelve_hours_even_without_a_warning() {
        assert_eq!(action(MAX_TOTAL, HOUR, true), CapAction::Stop);
        assert_eq!(action(MAX_TOTAL, PART, false), CapAction::Stop);
        assert_eq!(
            action(MAX_TOTAL + Duration::from_secs(90), Duration::ZERO, false),
            CapAction::Stop
        );
    }

    #[test]
    fn a_shorter_part_length_rolls_sooner() {
        let two_minutes = Duration::from_secs(120);
        assert_eq!(
            cap_action(two_minutes, two_minutes, two_minutes, false),
            CapAction::Roll
        );
    }

    /// The Meetings page tells, in every language, when a meeting rolls
    /// into a new part and when it stops.
    #[test]
    fn the_meetings_page_states_the_part_length_and_the_stop() {
        let locales = [
            include_str!("../../../src/i18n/locales/en/translation.json"),
            include_str!("../../../src/i18n/locales/fr/translation.json"),
            include_str!("../../../src/i18n/locales/de/translation.json"),
            include_str!("../../../src/i18n/locales/es/translation.json"),
            include_str!("../../../src/i18n/locales/it/translation.json"),
            include_str!("../../../src/i18n/locales/pt/translation.json"),
        ];
        let hours = |limit: Duration| (limit.as_secs() / 3600).to_string();
        for locale in locales {
            let json: serde_json::Value = serde_json::from_str(locale).unwrap();
            let text = json["meetings"]["recording"]["description"]
                .as_str()
                .unwrap();
            let numbers: Vec<&str> = text
                .split(|c: char| !c.is_ascii_digit())
                .filter(|number| !number.is_empty())
                .collect();
            assert_eq!(numbers, [hours(PART), hours(MAX_TOTAL)], "{text}");
        }
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
