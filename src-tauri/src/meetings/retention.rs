//! What retention deletes: what was set aside long enough ago, and the audio
//! of meetings older than the user chose to keep it.

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};

use super::store::{AudioState, MeetingStatus, SummaryStatus};
use crate::settings::MeetingAudioRetention;

/// What a reset library (`meetings.corrupt-`, `meetings.db.corrupt-`) or the
/// folders no meeting points to (`meetings.orphaned-`) leave next to the
/// library, each followed by the time it was set aside. No screen lists them.
pub const SET_ASIDE_PREFIXES: [&str; 3] = [
    "meetings.orphaned-",
    "meetings.corrupt-",
    "meetings.db.corrupt-",
];

/// Long enough to notice a lost library and copy its recordings back.
pub const SET_ASIDE_DAYS: i64 = 30;

/// When `name` was set aside, read from the name: a copy or a move keeps it,
/// where it would change the modification time. `None` for any other name.
fn set_aside_at(name: &str) -> Option<DateTime<Utc>> {
    let stamp = SET_ASIDE_PREFIXES
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))?;
    // A database moved aside takes its journal files along.
    let stamp = ["-wal", "-shm"]
        .iter()
        .find_map(|suffix| stamp.strip_suffix(suffix))
        .unwrap_or(stamp);
    NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S")
        .ok()
        .map(|at| at.and_utc())
}

/// Whether `name` is something set aside next to the library.
pub fn is_set_aside(name: &str) -> bool {
    set_aside_at(name).is_some()
}

/// The names set aside more than `SET_ASIDE_DAYS` before `now`.
pub fn expired_set_asides<'a>(
    names: impl IntoIterator<Item = &'a str>,
    now: DateTime<Utc>,
) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| {
            set_aside_at(name).is_some_and(|at| now - at > TimeDelta::days(SET_ASIDE_DAYS))
        })
        .map(str::to_string)
        .collect()
}

/// What retention needs to know of a meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeetingAudio {
    pub id: i64,
    pub started_at_ms: i64,
    pub status: MeetingStatus,
    pub summary: SummaryStatus,
    pub audio: AudioState,
}

fn days(retention: MeetingAudioRetention) -> Option<i64> {
    match retention {
        MeetingAudioRetention::Keep => None,
        MeetingAudioRetention::Days7 => Some(7),
        MeetingAudioRetention::Days30 => Some(30),
        MeetingAudioRetention::Days90 => Some(90),
    }
}

/// The meetings whose audio `retention` deletes at `now_ms`: finished ones,
/// summary included, that started more than its days ago. A meeting still
/// processing, interrupted or failed keeps its audio, which retrying needs.
pub fn audio_to_delete(
    meetings: &[MeetingAudio],
    now_ms: i64,
    retention: MeetingAudioRetention,
) -> Vec<i64> {
    let Some(days) = days(retention) else {
        return Vec::new();
    };
    let kept_ms = TimeDelta::days(days).num_milliseconds();
    meetings
        .iter()
        .filter(|meeting| {
            meeting.status == MeetingStatus::Ready
                && meeting.summary != SummaryStatus::Pending
                && meeting.audio != AudioState::Deleted
                && now_ms.saturating_sub(meeting.started_at_ms) > kept_ms
        })
        .map(|meeting| meeting.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 24 * 3600 * 1000;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn stamp(days_ago: i64) -> String {
        (now() - TimeDelta::days(days_ago))
            .format("%Y%m%d-%H%M%S")
            .to_string()
    }

    #[test]
    fn only_what_was_set_aside_over_30_days_ago_expires() {
        let old = stamp(31);
        let recent = stamp(29);
        let names = [
            format!("meetings.orphaned-{old}"),
            format!("meetings.corrupt-{old}"),
            format!("meetings.db.corrupt-{old}"),
            format!("meetings.db.corrupt-{old}-wal"),
            format!("meetings.db.corrupt-{old}-shm"),
            format!("meetings.orphaned-{recent}"),
            format!("meetings.db.corrupt-{recent}"),
            "meetings.orphaned-later".to_string(),
            format!("meetings.orphaned-{old}-copy"),
            "meetings.corrupt".to_string(),
            format!("recordings.corrupt-{old}"),
            format!("history.db.corrupt-{old}"),
            "meetings".to_string(),
            "meetings.db".to_string(),
        ];

        let expired = expired_set_asides(names.iter().map(String::as_str), now());

        assert_eq!(
            expired,
            [
                format!("meetings.orphaned-{old}"),
                format!("meetings.corrupt-{old}"),
                format!("meetings.db.corrupt-{old}"),
                format!("meetings.db.corrupt-{old}-wal"),
                format!("meetings.db.corrupt-{old}-shm"),
            ]
        );
    }

    fn meeting(
        status: MeetingStatus,
        summary: SummaryStatus,
        audio: AudioState,
        days_ago: i64,
    ) -> MeetingAudio {
        MeetingAudio {
            id: 1,
            started_at_ms: now().timestamp_millis() - days_ago * DAY_MS,
            status,
            summary,
            audio,
        }
    }

    fn deleted(meeting: MeetingAudio, retention: MeetingAudioRetention) -> bool {
        !audio_to_delete(&[meeting], now().timestamp_millis(), retention).is_empty()
    }

    #[test]
    fn audio_goes_only_from_finished_meetings_older_than_the_chosen_days() {
        use AudioState::{Compressed, Deleted, Tracks};
        use MeetingAudioRetention::{Days30, Days7, Days90, Keep};
        use MeetingStatus::{Failed, Interrupted, Processing, Ready, Recording};
        use SummaryStatus::Pending;

        let ready = |days_ago| meeting(Ready, SummaryStatus::Ready, Compressed, days_ago);
        assert!(deleted(ready(8), Days7));
        assert!(!deleted(ready(6), Days7));
        assert!(!deleted(ready(29), Days30));
        assert!(deleted(ready(31), Days30));
        assert!(!deleted(ready(89), Days90));
        assert!(deleted(ready(91), Days90));
        assert!(!deleted(ready(3650), Keep));

        // Tracks that never compressed go too; a failed summary is settled.
        assert!(deleted(
            meeting(Ready, SummaryStatus::Ready, Tracks, 8),
            Days7
        ));
        assert!(deleted(
            meeting(Ready, SummaryStatus::Failed, Compressed, 8),
            Days7
        ));

        // Whatever still needs its audio keeps it, however old.
        for status in [Failed, Interrupted, Processing, Recording] {
            assert!(
                !deleted(
                    meeting(status, SummaryStatus::Ready, Compressed, 100),
                    Days7
                ),
                "{status:?}"
            );
        }
        assert!(!deleted(meeting(Ready, Pending, Compressed, 100), Days7));
        assert!(!deleted(
            meeting(Ready, SummaryStatus::Ready, Deleted, 100),
            Days7
        ));
    }

    #[test]
    fn several_meetings_are_picked_by_id() {
        let mut old = meeting(
            MeetingStatus::Ready,
            SummaryStatus::Ready,
            AudioState::Compressed,
            40,
        );
        old.id = 4;
        let mut recent = old;
        recent.id = 5;
        recent.started_at_ms = now().timestamp_millis() - DAY_MS;
        let mut failed = old;
        failed.id = 6;
        failed.status = MeetingStatus::Failed;

        assert_eq!(
            audio_to_delete(
                &[old, recent, failed],
                now().timestamp_millis(),
                MeetingAudioRetention::Days30
            ),
            [4]
        );
    }
}
