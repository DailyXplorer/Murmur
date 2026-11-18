use anyhow::Result;
use rusqlite::{params, TransactionBehavior};

use super::MeetingsStore;
use crate::meetings::merge::ME;
use crate::meetings::summary::{same_name, Named, SummaryError};

impl MeetingsStore {
    /// Saves the summary, the title it gives the meeting unless the user
    /// named it, and the names it read for speakers: all of it, or nothing.
    /// Returns false, saving nothing, when the meeting no longer waits for
    /// it: transcribed again or re-analysed meanwhile, its summary is of a
    /// transcript that is gone.
    ///
    /// A speaker takes its name only while it shows its default one, and
    /// while no other speaker of the meeting shows that name or has it as a
    /// default, as the rows are now: the user may have renamed a speaker
    /// while the summary was written. The user's own track is never named.
    pub fn save_summary(
        &self,
        id: i64,
        title: Option<&str>,
        body: &str,
        named: &[Named],
    ) -> Result<bool> {
        let mut connection = self.connection()?;
        // The names are read, then written: the write lock is taken first, so
        // no rename lands in between.
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let updated = tx.execute(
            "UPDATE meetings SET summary_md = ?3, summary_status = 'ready', summary_error = NULL,
                 title = CASE WHEN title_edited = 0 AND ?2 IS NOT NULL THEN ?2 ELSE title END
             WHERE id = ?1 AND status = 'ready' AND summary_status = 'pending'",
            params![id, title, body],
        )?;
        if updated == 0 {
            return Ok(false);
        }
        let mut taken = 0;
        for Named { key, name } in named {
            // Compared here and not in SQL, whose NOCASE folds ASCII only.
            let others: Vec<(String, String)> = tx
                .prepare(
                    "SELECT display_name, default_name FROM meeting_speakers
                     WHERE meeting_id = ?1 AND key <> ?2",
                )?
                .query_map(params![id, key], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            if others
                .iter()
                .any(|(shown, default)| same_name(shown, name) || same_name(default, name))
            {
                continue;
            }
            taken += tx.execute(
                "UPDATE meeting_speakers SET display_name = ?3
                 WHERE meeting_id = ?1 AND key = ?2 AND key <> ?4
                   AND display_name = default_name",
                params![id, key, name, ME],
            )?;
        }
        tx.commit()?;
        if taken > 0 {
            log::info!("Meeting {id}: {taken} speakers named from the conversation");
        }
        Ok(true)
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
