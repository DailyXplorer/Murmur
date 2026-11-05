use anyhow::Result;
use rusqlite::params;

use super::MeetingsStore;
use crate::meetings::summary::SummaryError;

impl MeetingsStore {
    /// Saves the summary, and the title it gives the meeting unless the
    /// user named it. Returns false when the meeting no longer waits for
    /// it: transcribed again or re-analysed meanwhile, its summary is of a
    /// transcript that is gone.
    pub fn save_summary(&self, id: i64, title: Option<&str>, body: &str) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET summary_md = ?3, summary_status = 'ready', summary_error = NULL,
                 title = CASE WHEN title_edited = 0 AND ?2 IS NOT NULL THEN ?2 ELSE title END
             WHERE id = ?1 AND status = 'ready' AND summary_status = 'pending'",
            params![id, title, body],
        )?;
        Ok(updated > 0)
    }

    /// Returns false, like `save_summary`, when the meeting no longer waits
    /// for this summary.
    pub fn summary_failed(&self, id: i64, error: SummaryError) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET summary_status = 'failed', summary_error = ?2
             WHERE id = ?1 AND status = 'ready' AND summary_status = 'pending'",
            params![id, error.as_db()],
        )?;
        Ok(updated > 0)
    }

    /// Makes a failed summary wait to be written again. Returns false when
    /// it did not fail, or when the meeting is not Ready: only a Ready
    /// meeting gets its summary written.
    pub fn retry_summary(&self, id: i64) -> Result<bool> {
        let updated = self.connection()?.execute(
            "UPDATE meetings SET summary_status = 'pending', summary_error = NULL
             WHERE id = ?1 AND status = 'ready' AND summary_status = 'failed'",
            params![id],
        )?;
        Ok(updated > 0)
    }
}
