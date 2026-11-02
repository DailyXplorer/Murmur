//! A small, invented data directory for the tests of `murmur mcp` (the unit
//! tests and `tests/mcp_stdio.rs`, which runs the built binary). No real
//! meeting or dictation is read or written.

use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::Path;

use crate::managers::history;
use crate::meetings::store::MeetingsStore;

const ROADMAP_STARTED_MS: i64 = 1_772_445_600_000;
const BUDGET_STARTED_MS: i64 = 1_773_070_200_000;
const UNTITLED_STARTED_MS: i64 = 1_773_133_200_000;

/// Writes the settings file, `meetings.db` (three meetings) and `history.db`
/// (two dictations) of an app whose user turned the server `mcp_enabled`
/// and, with `include_dictations`, dictations too.
///
/// Meeting 1 is "Roadmap review" with a renamed speaker, meeting 2 is a
/// French "Réunion budget", meeting 3 is still processing and has no title.
/// The dictations carry a file name and an uncleaned text the server must
/// never return.
pub fn seed_sample_data_dir(dir: &Path, mcp_enabled: bool, include_dictations: bool) -> Result<()> {
    write_settings(dir, mcp_enabled, include_dictations)?;

    let meetings = dir.join("meetings.db");
    MeetingsStore::open(meetings.clone(), chrono::Utc::now());
    let conn = Connection::open(&meetings)?;
    conn.execute_batch(&format!(
        "INSERT INTO meetings (id, started_at_ms, duration_ms, capture_mode, status, title, summary_md, summary_status)
         VALUES (1, {ROADMAP_STARTED_MS}, 1800000, 'both', 'ready', 'Roadmap review',
                 '## Decisions\n- [[s1]] owns the launch checklist.\n- The budget is approved.', 'ready');
         INSERT INTO meetings (id, started_at_ms, duration_ms, capture_mode, status, title, summary_md, summary_status)
         VALUES (2, {BUDGET_STARTED_MS}, 900000, 'mic', 'ready', 'Réunion budget',
                 'Résumé du budget avec [[me]].', 'ready');
         INSERT INTO meetings (id, started_at_ms, duration_ms, capture_mode, status)
         VALUES (3, {UNTITLED_STARTED_MS}, 60000, 'mic', 'processing');
         INSERT INTO meeting_speakers (meeting_id, key, default_name, display_name, color_index)
         VALUES (1, 'me', 'Me', 'Me', 0), (1, 's1', 'Speaker 1', 'Product lead', 1),
                (2, 'me', 'Me', 'Me', 0);
         INSERT INTO meeting_turns (meeting_id, seq, speaker_key, start_ms, end_ms, text, low_confidence)
         VALUES (1, 0, 'me', 0, 6000, 'Let us review the roadmap for next quarter.', 0),
                (1, 1, 's1', 6000, 12000, 'The launch checklist is nearly done.', 1),
                (1, 2, 'me', 12000, 15000, 'The budget is approved.', 0),
                (2, 0, 'me', 0, 8000, 'Nous avons parlé du budget et des recrutements.', 0);"
    ))?;

    let history_path = dir.join("history.db");
    history::init_database(&history_path)?;
    let conn = Connection::open(&history_path)?;
    for (timestamp, text) in [
        (
            1_773_000_000_i64,
            "Remind me to send the invoice on Friday.",
        ),
        (1_773_000_100, "Call the plumber tomorrow morning."),
    ] {
        conn.execute(
            "INSERT INTO transcription_history
                 (file_name, timestamp, saved, title, transcription_text, raw_text)
             VALUES (?1, ?2, 0, 'Recording', ?3, 'raw uncleaned words')",
            params![
                format!("private-recording-{timestamp}.wav"),
                timestamp,
                text
            ],
        )?;
    }
    Ok(())
}

/// The settings file as the app writes it, with the two switches.
pub fn write_settings(dir: &Path, mcp_enabled: bool, include_dictations: bool) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let store = serde_json::json!({
        "settings": {"mcp_enabled": mcp_enabled, "mcp_include_dictations": include_dictations}
    });
    std::fs::write(dir.join("settings_store.json"), store.to_string())?;
    Ok(())
}
