use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

pub const WPM_WINDOW_DAYS: i64 = 30;
pub const HEATMAP_WEEKS: i64 = 20;
/// Very short clips (an accidental tap) carry no useful speaking rate.
const MIN_SPEAKING_MS: u32 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DictationStat {
    /// Unix seconds.
    pub at: i64,
    pub words: u32,
    /// Unknown for backfilled rows whose recording was already deleted.
    pub speaking_ms: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Streaks {
    pub current: u32,
    pub longest: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct HeatmapDay {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    pub count: u32,
    /// Intensity bucket from 0 (no dictation) to 4 (busiest day in range).
    pub level: u8,
}

/// A Chinese or Japanese character: Han with its marks 々 〆 〇, Hiragana
/// or Katakana.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3005..=0x3007   // 々, 〆, 〇
        | 0x3040..=0x30FF // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0x20000..=0x2FFFF)
}

/// Counts words the way a reader would: whitespace-separated tokens that
/// contain a letter or digit, and each Chinese or Japanese character on its
/// own because those scripts do not separate words with spaces.
pub fn count_words(text: &str) -> u32 {
    text.split_whitespace()
        .map(|token| {
            let cjk = token.chars().filter(|c| is_cjk(*c)).count() as u32;
            let other = token.chars().any(|c| c.is_alphanumeric() && !is_cjk(c));
            cjk + u32::from(other)
        })
        .sum()
}

/// Total words divided by total recording minutes for dictations at or after
/// `since`. Weighting by duration keeps one short burst from skewing the rate.
pub fn words_per_minute(dictations: &[DictationStat], since: i64) -> Option<u32> {
    let (words, ms) = dictations
        .iter()
        .filter(|d| d.at >= since)
        .filter_map(|d| {
            d.speaking_ms
                .filter(|ms| *ms >= MIN_SPEAKING_MS)
                .map(|ms| (u64::from(d.words), u64::from(ms)))
        })
        .fold((0u64, 0u64), |(w, t), (dw, dt)| (w + dw, t + dt));
    (ms > 0).then(|| ((words as f64) * 60_000.0 / ms as f64).round() as u32)
}

/// Buckets each timestamp into the calendar day it fell on in `tz`. The
/// offset is resolved per timestamp so a DST change never shifts a day.
pub fn dictations_per_day<Tz: TimeZone>(
    timestamps: impl IntoIterator<Item = i64>,
    tz: &Tz,
) -> BTreeMap<NaiveDate, u32> {
    let mut days = BTreeMap::new();
    for ts in timestamps {
        if let Some(utc) = DateTime::from_timestamp(ts, 0) {
            *days.entry(utc.with_timezone(tz).date_naive()).or_insert(0) += 1;
        }
    }
    days
}

/// The current streak counts back from today, or from yesterday when today
/// has no dictation yet, so a streak only breaks once a full day is missed.
pub fn streaks(days: &BTreeMap<NaiveDate, u32>, today: NaiveDate) -> Streaks {
    let mut longest = 0;
    let mut run = 0;
    let mut previous: Option<NaiveDate> = None;
    for (&day, &count) in days {
        if count == 0 {
            continue;
        }
        run = match previous {
            Some(p) if p.succ_opt() == Some(day) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        previous = Some(day);
    }

    let active = |day: NaiveDate| days.get(&day).is_some_and(|c| *c > 0);
    let mut cursor = if active(today) {
        today
    } else {
        today - Duration::days(1)
    };
    let mut current = 0;
    while active(cursor) {
        current += 1;
        cursor -= Duration::days(1);
    }

    Streaks { current, longest }
}

/// Days from the Sunday that starts the oldest week through `today`, so the
/// UI can lay them out as weekday rows by week columns.
pub fn heatmap(days: &BTreeMap<NaiveDate, u32>, today: NaiveDate, weeks: i64) -> Vec<HeatmapDay> {
    let week_start = today - Duration::days(i64::from(today.weekday().num_days_from_sunday()));
    let first = week_start - Duration::weeks(weeks - 1);
    let range: Vec<(NaiveDate, u32)> = first
        .iter_days()
        .take_while(|day| *day <= today)
        .map(|day| (day, days.get(&day).copied().unwrap_or(0)))
        .collect();
    let max = range.iter().map(|(_, c)| *c).max().unwrap_or(0);
    range
        .into_iter()
        .map(|(day, count)| HeatmapDay {
            date: day.format("%Y-%m-%d").to_string(),
            count,
            level: if count == 0 {
                0
            } else {
                (count * 4).div_ceil(max).clamp(1, 4) as u8
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use chrono_tz::{America::New_York, Asia::Tokyo};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
            .timestamp()
    }

    fn days(list: &[(NaiveDate, u32)]) -> BTreeMap<NaiveDate, u32> {
        list.iter().copied().collect()
    }

    #[test]
    fn counts_words_across_scripts() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("Hello, world! It's 2 PM — ok?"), 6);
        assert_eq!(count_words("  spaced\tout\nwords "), 3);
        assert_eq!(count_words("你好世界"), 4);
        assert_eq!(count_words("今日は Murmur です"), 6);
        assert_eq!(count_words("안녕하세요 세계"), 2);
    }

    #[test]
    fn words_per_minute_weights_by_recording_time() {
        let dictations = [
            DictationStat {
                at: 100,
                words: 60,
                speaking_ms: Some(30_000),
            },
            DictationStat {
                at: 200,
                words: 90,
                speaking_ms: Some(60_000),
            },
        ];
        assert_eq!(words_per_minute(&dictations, 0), Some(100));
    }

    #[test]
    fn words_per_minute_ignores_old_short_and_unknown_durations() {
        let dictations = [
            DictationStat {
                at: 10,
                words: 500,
                speaking_ms: Some(60_000),
            },
            DictationStat {
                at: 100,
                words: 50,
                speaking_ms: Some(500),
            },
            DictationStat {
                at: 100,
                words: 50,
                speaking_ms: None,
            },
            DictationStat {
                at: 100,
                words: 75,
                speaking_ms: Some(30_000),
            },
        ];
        assert_eq!(words_per_minute(&dictations, 50), Some(150));
        assert_eq!(words_per_minute(&dictations[1..3], 50), None);
        assert_eq!(words_per_minute(&[], 0), None);
    }

    #[test]
    fn days_are_local_dates_across_spring_forward() {
        // US DST began 2026-03-08. A fixed -04:00 offset would put the first
        // dictation on March 8 instead of March 7.
        let per_day = dictations_per_day(
            [
                utc(2026, 3, 8, 4, 30), // Mar 7 23:30 EST
                utc(2026, 3, 9, 3, 30), // Mar 8 23:30 EDT
                utc(2026, 3, 9, 4, 30), // Mar 9 00:30 EDT
            ],
            &New_York,
        );
        assert_eq!(
            per_day,
            days(&[
                (date(2026, 3, 7), 1),
                (date(2026, 3, 8), 1),
                (date(2026, 3, 9), 1)
            ])
        );
        assert_eq!(
            streaks(&per_day, date(2026, 3, 9)),
            Streaks {
                current: 3,
                longest: 3
            }
        );
    }

    #[test]
    fn repeated_hour_at_fall_back_stays_on_one_day() {
        // 01:30 happens twice on 2026-11-01 in New York.
        let per_day = dictations_per_day(
            [utc(2026, 11, 1, 5, 30), utc(2026, 11, 1, 6, 30)],
            &New_York,
        );
        assert_eq!(per_day, days(&[(date(2026, 11, 1), 2)]));
    }

    #[test]
    fn same_instant_lands_on_the_local_day_of_each_timezone() {
        let instant = utc(2026, 5, 1, 20, 0);
        assert_eq!(
            dictations_per_day([instant], &New_York),
            days(&[(date(2026, 5, 1), 1)])
        );
        assert_eq!(
            dictations_per_day([instant], &Tokyo),
            days(&[(date(2026, 5, 2), 1)])
        );
    }

    #[test]
    fn current_streak_survives_until_a_full_day_is_missed() {
        let per_day = days(&[
            (date(2026, 1, 1), 2),
            (date(2026, 1, 2), 1),
            (date(2026, 1, 3), 1),
            (date(2026, 1, 4), 1),
            (date(2026, 1, 10), 1),
            (date(2026, 1, 11), 3),
        ]);
        assert_eq!(
            streaks(&per_day, date(2026, 1, 11)),
            Streaks {
                current: 2,
                longest: 4
            }
        );
        assert_eq!(
            streaks(&per_day, date(2026, 1, 12)),
            Streaks {
                current: 2,
                longest: 4
            }
        );
        assert_eq!(
            streaks(&per_day, date(2026, 1, 13)),
            Streaks {
                current: 0,
                longest: 4
            }
        );
        assert_eq!(
            streaks(&BTreeMap::new(), date(2026, 1, 1)),
            Streaks::default()
        );
    }

    #[test]
    fn heatmap_starts_on_a_sunday_and_ends_today() {
        // 2026-10-15 is a Thursday.
        let today = date(2026, 10, 15);
        let per_day = days(&[
            (date(2026, 10, 15), 8),
            (date(2026, 10, 14), 2),
            (date(2026, 10, 13), 1),
            (date(2026, 1, 1), 50),
        ]);
        let cells = heatmap(&per_day, today, 20);

        assert_eq!(cells.len(), 19 * 7 + 5);
        assert_eq!(cells.first().unwrap().date, "2026-05-31");
        assert_eq!(
            NaiveDate::parse_from_str(&cells[0].date, "%Y-%m-%d")
                .unwrap()
                .weekday(),
            chrono::Weekday::Sun
        );
        let tail: Vec<(&str, u32, u8)> = cells[cells.len() - 3..]
            .iter()
            .map(|c| (c.date.as_str(), c.count, c.level))
            .collect();
        assert_eq!(
            tail,
            [
                ("2026-10-13", 1, 1),
                ("2026-10-14", 2, 1),
                ("2026-10-15", 8, 4)
            ]
        );
        assert_eq!(cells[0].level, 0);
    }
}
