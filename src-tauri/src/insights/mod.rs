//! Local dictation statistics for the Insights page. Stored in `insights.db`
//! next to `history.db` so history retention never erases them. No dictated
//! text is stored here, only counts, timestamps and the receiving app.

pub mod categories;
pub mod frontmost;
pub mod host;
pub mod stats;

use anyhow::Result;
use categories::{categorize, AppCategory};
use chrono::{Local, TimeZone, Utc};
use log::{error, info, warn};
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use stats::{DictationStat, HeatmapDay, Streaks};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::managers::history::HistoryManager;

static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE dictations (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at INTEGER NOT NULL,
        word_count INTEGER NOT NULL,
        speaking_ms INTEGER,
        bundle_id TEXT,
        host TEXT,
        dictionary_fixes INTEGER NOT NULL DEFAULT 0,
        history_id INTEGER UNIQUE
    );
    CREATE INDEX dictations_created_at ON dictations(created_at);
    CREATE TABLE corrections (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at INTEGER NOT NULL,
        word_count INTEGER NOT NULL
    );
    CREATE TABLE backfills (
        source TEXT PRIMARY KEY,
        completed_at INTEGER NOT NULL
    );",
    ),
    // Drops rows that counted every word changed after a paste, which replies
    // typed into a chat field inflated. A correction is one learned word.
    M::up("DELETE FROM corrections;"),
];

const HISTORY_BACKFILL: &str = "history";

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
pub struct InsightsUpdated;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CategoryUsage {
    pub category: AppCategory,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct InsightsSummary {
    pub total_dictations: u32,
    pub total_words: u32,
    /// Over the last [`stats::WPM_WINDOW_DAYS`] days; `None` without timed dictations.
    pub words_per_minute: Option<u32>,
    /// Against typing at [`stats::TYPING_WPM`], over the same dictations as `total_words`.
    pub minutes_saved: u32,
    /// Non-empty categories, most used first.
    pub categories: Vec<CategoryUsage>,
    /// Dictations whose receiving app is known. Rows backfilled from history have none.
    pub categorized_dictations: u32,
    pub apps_used: u32,
    pub streaks: Streaks,
    pub heatmap: Vec<HeatmapDay>,
}

/// A completed history row, the only source for the one-time backfill.
pub struct HistoryRow {
    pub id: i64,
    pub timestamp: i64,
    pub text: String,
    pub file_name: String,
}

struct NewDictation<'a> {
    at: i64,
    words: u32,
    speaking_ms: Option<u32>,
    bundle_id: Option<&'a str>,
    host: Option<&'a str>,
    dictionary_fixes: u32,
}

fn read_history_rows(history_db: &std::path::Path) -> Result<Vec<HistoryRow>> {
    let conn = Connection::open(history_db)?;
    let rows = conn
        .prepare(
            "SELECT id, timestamp, transcription_text, file_name
             FROM transcription_history WHERE transcription_text != ''",
        )?
        .query_map([], |row| {
            Ok(HistoryRow {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                text: row.get(2)?,
                file_name: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    Migrations::new(MIGRATIONS.to_vec()).to_latest(conn)?;
    Ok(())
}

fn insert_dictation(conn: &Connection, d: &NewDictation) -> Result<()> {
    conn.execute(
        "INSERT INTO dictations (created_at, word_count, speaking_ms, bundle_id, host, dictionary_fixes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![d.at, d.words, d.speaking_ms, d.bundle_id, d.host, d.dictionary_fixes],
    )?;
    Ok(())
}

/// Returns whether a row was written; a correction of zero words is ignored.
fn insert_correction(conn: &Connection, at: i64, words: u32) -> Result<bool> {
    if words == 0 {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO corrections (created_at, word_count) VALUES (?1, ?2)",
        params![at, words],
    )?;
    Ok(true)
}

/// Imports dates, word counts and (when the WAV still exists) durations from
/// history once. Rows and the completion marker commit together, so a crash
/// mid-way retries cleanly and a finished backfill never runs again. History
/// is read only while the backfill is pending, and an unreadable history
/// postpones it to a later launch instead of disabling insights.
fn backfill_from_history(
    conn: &mut Connection,
    read_rows: impl FnOnce() -> Result<Vec<HistoryRow>>,
    duration_ms: impl Fn(&str) -> Option<u32>,
) -> Result<usize> {
    let tx = conn.transaction()?;
    let done: bool = tx.query_row(
        "SELECT COUNT(*) > 0 FROM backfills WHERE source = ?1",
        params![HISTORY_BACKFILL],
        |row| row.get(0),
    )?;
    if done {
        return Ok(0);
    }
    let rows = match read_rows() {
        Ok(rows) => rows,
        Err(e) => {
            warn!("History is unreadable; postponing the insights backfill: {e}");
            return Ok(0);
        }
    };
    let mut imported = 0;
    for row in rows.iter().filter(|row| !row.text.trim().is_empty()) {
        imported += tx.execute(
            "INSERT OR IGNORE INTO dictations (created_at, word_count, speaking_ms, history_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                row.timestamp,
                stats::count_words(&row.text),
                duration_ms(&row.file_name),
                row.id
            ],
        )?;
    }
    tx.execute(
        "INSERT INTO backfills (source, completed_at) VALUES (?1, ?2)",
        params![HISTORY_BACKFILL, Utc::now().timestamp()],
    )?;
    tx.commit()?;
    Ok(imported)
}

fn summary_with_conn<Tz: TimeZone>(
    conn: &Connection,
    now: i64,
    tz: &Tz,
) -> Result<InsightsSummary> {
    struct Row {
        stat: DictationStat,
        bundle_id: Option<String>,
        host: Option<String>,
    }
    let rows = conn
        .prepare(
            "SELECT created_at, word_count, speaking_ms, bundle_id, host
             FROM dictations ORDER BY created_at",
        )?
        .query_map([], |row| {
            Ok(Row {
                stat: DictationStat {
                    at: row.get(0)?,
                    words: row.get(1)?,
                    speaking_ms: row.get(2)?,
                },
                bundle_id: row.get(3)?,
                host: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let dictation_stats: Vec<DictationStat> = rows.iter().map(|r| r.stat).collect();
    let mut category_counts: BTreeMap<AppCategory, u32> = BTreeMap::new();
    let mut apps = HashSet::new();
    for row in &rows {
        if let Some(bundle_id) = row.bundle_id.as_deref() {
            *category_counts
                .entry(categorize(bundle_id, row.host.as_deref()))
                .or_insert(0) += 1;
            apps.insert(bundle_id);
        }
    }
    let mut categories: Vec<CategoryUsage> = category_counts
        .into_iter()
        .map(|(category, count)| CategoryUsage { category, count })
        .collect();
    categories.sort_by_key(|usage| std::cmp::Reverse(usage.count));

    let today = Utc
        .timestamp_opt(now, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(tz)
        .date_naive();
    let per_day = stats::dictations_per_day(dictation_stats.iter().map(|d| d.at), tz);

    Ok(InsightsSummary {
        total_dictations: rows.len() as u32,
        total_words: dictation_stats.iter().map(|d| d.words).sum(),
        words_per_minute: stats::words_per_minute(
            &dictation_stats,
            now - stats::WPM_WINDOW_DAYS * 86_400,
        ),
        minutes_saved: stats::minutes_saved(&dictation_stats),
        categorized_dictations: categories.iter().map(|c| c.count).sum(),
        categories,
        apps_used: apps.len() as u32,
        streaks: stats::streaks(&per_day, today),
        heatmap: stats::heatmap(&per_day, today, stats::HEATMAP_WEEKS),
    })
}

pub struct InsightsManager {
    app_handle: AppHandle,
    db_path: PathBuf,
}

impl InsightsManager {
    /// Opens `insights.db`, migrates it, and runs the one-time history backfill.
    /// Must be created after the history manager has migrated `history.db`.
    pub fn new(app_handle: &AppHandle, history: &HistoryManager) -> Result<Self> {
        let db_path = app_handle.path().app_data_dir()?.join("insights.db");
        let mut conn = Connection::open(&db_path)?;
        migrate(&mut conn)?;

        let history_db = db_path.with_file_name("history.db");
        let read_rows = || read_history_rows(&history_db);
        let imported = backfill_from_history(&mut conn, read_rows, |file_name| {
            let path = history.resolve_persisted_recording_path(file_name).ok()?;
            let reader = hound::WavReader::open(path).ok()?;
            let rate = u64::from(reader.spec().sample_rate.max(1));
            u32::try_from(u64::from(reader.duration()) * 1000 / rate).ok()
        })?;
        if imported > 0 {
            info!("Backfilled {imported} dictations into insights");
        }

        Ok(Self {
            app_handle: app_handle.clone(),
            db_path,
        })
    }

    fn connection(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    fn notify(&self) {
        if let Err(e) = InsightsUpdated.emit(&self.app_handle) {
            error!("Failed to emit insights-updated event: {e}");
        }
    }

    pub fn summary(&self) -> Result<InsightsSummary> {
        summary_with_conn(&self.connection()?, Utc::now().timestamp(), &Local)
    }

    /// Records misheard words the user corrected after a paste, one per
    /// word the dictionary learned from it.
    pub fn record_correction(&self, words_corrected: u32) -> Result<()> {
        if insert_correction(&self.connection()?, Utc::now().timestamp(), words_corrected)? {
            self.notify();
        }
        Ok(())
    }

    fn record_dictation(
        &self,
        words: u32,
        sample_count: usize,
        dictionary_fixes: u32,
    ) -> Result<()> {
        let target = frontmost::current();
        let speaking_ms = (sample_count as u64 * 1000 / u64::from(TARGET_SAMPLE_RATE)) as u32;
        insert_dictation(
            &self.connection()?,
            &NewDictation {
                at: Utc::now().timestamp(),
                words,
                speaking_ms: Some(speaking_ms),
                bundle_id: target.as_ref().map(|t| t.bundle_id.as_str()),
                host: target.as_ref().and_then(|t| t.host.as_deref()),
                dictionary_fixes,
            },
        )?;
        self.notify();
        Ok(())
    }
}

/// Records a delivered dictation off the caller's thread. Called right after
/// the paste, so the frontmost app is the one that received the text.
pub fn record_dictation(app: &AppHandle, text: &str, sample_count: usize, dictionary_fixes: u32) {
    let Some(manager) = app.try_state::<Arc<InsightsManager>>() else {
        return;
    };
    let manager = Arc::clone(&manager);
    let words = stats::count_words(text);
    std::thread::spawn(move || {
        if let Err(e) = manager.record_dictation(words, sample_count, dictionary_fixes) {
            error!("Failed to record dictation insights: {e}");
        }
    });
}

#[tauri::command]
#[specta::specta]
pub fn get_insights(app: AppHandle) -> Result<InsightsSummary, String> {
    app.try_state::<Arc<InsightsManager>>()
        .ok_or_else(|| "Insights are unavailable".to_string())?
        .summary()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::Paris;

    fn migrated() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    fn history_row(id: i64, timestamp: i64, text: &str) -> HistoryRow {
        HistoryRow {
            id,
            timestamp,
            text: text.to_string(),
            file_name: format!("murmur-{id}.wav"),
        }
    }

    fn ts(y: i32, m: u32, d: u32, h: u32) -> i64 {
        Paris
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn unreadable_history_postpones_the_backfill_without_failing() {
        let mut conn = migrated();
        let no_duration = |_: &str| None;

        let postponed = backfill_from_history(
            &mut conn,
            || Err(anyhow::anyhow!("no such table: transcription_history")),
            no_duration,
        );
        assert_eq!(postponed.unwrap(), 0);

        let rows = vec![history_row(1, ts(2026, 10, 13, 9), "one two three")];
        assert_eq!(
            backfill_from_history(&mut conn, || Ok(rows), no_duration).unwrap(),
            1
        );
    }

    #[test]
    fn migrations_are_valid() {
        Migrations::new(MIGRATIONS.to_vec()).validate().unwrap();
    }

    #[test]
    fn corrections_counted_as_raw_edits_are_dropped_on_upgrade() {
        let mut conn = Connection::open_in_memory().unwrap();
        Migrations::new(MIGRATIONS[..1].to_vec())
            .to_latest(&mut conn)
            .unwrap();
        assert!(insert_correction(&conn, ts(2026, 10, 14, 9), 7).unwrap());

        migrate(&mut conn).unwrap();
        assert!(insert_correction(&conn, ts(2026, 10, 15, 9), 1).unwrap());

        let words: u32 = conn
            .query_row("SELECT SUM(word_count) FROM corrections", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(words, 1);
    }

    #[test]
    fn backfill_imports_completed_history_once() {
        let mut conn = migrated();
        let rows = vec![
            history_row(1, ts(2026, 10, 13, 9), "one two three"),
            history_row(2, ts(2026, 10, 14, 9), ""),
            history_row(3, ts(2026, 10, 15, 9), "four five"),
        ];
        let duration = |file: &str| (file == "murmur-1.wav").then_some(1_500);

        assert_eq!(
            backfill_from_history(&mut conn, || Ok(rows), duration).unwrap(),
            2
        );
        assert_eq!(
            backfill_from_history(
                &mut conn,
                || panic!("a finished backfill must not read history"),
                duration
            )
            .unwrap(),
            0
        );

        let summary = summary_with_conn(&conn, ts(2026, 10, 15, 12), &Paris).unwrap();
        assert_eq!(summary.total_dictations, 2);
        assert_eq!(summary.total_words, 5);
        assert_eq!(summary.words_per_minute, Some(120));
        assert_eq!(summary.categorized_dictations, 0);
        assert_eq!(
            summary.streaks,
            Streaks {
                current: 1,
                longest: 1
            }
        );
    }

    #[test]
    fn summary_combines_categories_and_streaks() {
        let conn = migrated();
        let now = ts(2026, 10, 15, 18);
        let dictation = |at, bundle_id, host, fixes| NewDictation {
            at,
            words: 30,
            speaking_ms: Some(12_000),
            bundle_id,
            host,
            dictionary_fixes: fixes,
        };
        let (slack, browser, mail) = if cfg!(target_os = "windows") {
            ("slack.exe", "chrome.exe", "olk.exe")
        } else {
            (
                "com.tinyspeck.slackmacgap",
                "com.apple.Safari",
                "com.apple.mail",
            )
        };
        for d in [
            dictation(ts(2026, 10, 13, 9), Some(slack), None, 0),
            dictation(
                ts(2026, 10, 14, 9),
                Some(browser),
                Some("mail.google.com"),
                2,
            ),
            dictation(ts(2026, 10, 15, 9), Some(mail), None, 1),
            dictation(ts(2026, 10, 15, 10), None, None, 0),
        ] {
            insert_dictation(&conn, &d).unwrap();
        }
        assert!(insert_correction(&conn, now, 3).unwrap());
        assert!(!insert_correction(&conn, now, 0).unwrap());

        let summary = summary_with_conn(&conn, now, &Paris).unwrap();

        assert_eq!(summary.total_words, 120);
        assert_eq!(summary.words_per_minute, Some(150));
        assert_eq!(
            summary.categories,
            [
                CategoryUsage {
                    category: AppCategory::Emails,
                    count: 2
                },
                CategoryUsage {
                    category: AppCategory::WorkMessages,
                    count: 1
                },
            ]
        );
        assert_eq!(summary.categorized_dictations, 3);
        assert_eq!(summary.apps_used, 3);
        assert_eq!(
            summary.streaks,
            Streaks {
                current: 3,
                longest: 3
            }
        );
        let last = summary.heatmap.last().unwrap();
        assert_eq!((last.date.as_str(), last.count), ("2026-10-15", 2));
    }

    #[test]
    fn time_saved_covers_every_dictation_like_total_words() {
        let conn = migrated();
        let now = ts(2026, 10, 15, 18);
        for at in [now - 90 * 86_400, now - 3_600] {
            let dictation = NewDictation {
                at,
                words: 200,
                speaking_ms: Some(60_000),
                bundle_id: None,
                host: None,
                dictionary_fixes: 0,
            };
            insert_dictation(&conn, &dictation).unwrap();
        }

        let summary = summary_with_conn(&conn, now, &Paris).unwrap();

        assert_eq!(summary.total_words, 400);
        assert_eq!(summary.minutes_saved, 8);
    }

    #[test]
    fn empty_database_reports_zeroes() {
        let conn = migrated();
        let summary = summary_with_conn(&conn, ts(2026, 10, 15, 12), &Paris).unwrap();
        assert_eq!(summary.total_words, 0);
        assert_eq!(summary.words_per_minute, None);
        assert_eq!(summary.minutes_saved, 0);
        assert!(summary.categories.is_empty());
        assert_eq!(summary.streaks, Streaks::default());
        assert!(summary.heatmap.iter().all(|d| d.count == 0 && d.level == 0));
    }
}
