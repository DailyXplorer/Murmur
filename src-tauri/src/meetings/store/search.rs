use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

use super::types::{MeetingHit, MeetingStatus};
use super::MeetingsStore;
use crate::search::{fts_terms, Snippet, HIT_END, HIT_START};

/// Where a meeting matched, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HitPlace {
    Title,
    Summary,
    Transcript,
}

struct Candidate {
    place: HitPlace,
    /// `bm25()` at that place: lower is more relevant.
    score: f64,
    marked_title: Option<String>,
    marked_summary: Option<String>,
    /// `(seq, rowid)` of the first matching turns.
    turns: Vec<(i64, i64)>,
}

/// Snippets a meeting hit shows at most.
const MAX_SNIPPETS: usize = 2;

/// Matching turns past which a search stops ranking them.
const MAX_TURN_HITS: i64 = 2_000;

/// The meetings holding every word of `query` somewhere: in the title, the
/// summary or any turn, not necessarily together. They are ranked by where
/// they matched (title, then summary, then transcript only), then
/// relevance, then newest. Read-only: it works on any connection to
/// `meetings.db`.
pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<MeetingHit>> {
    let terms = fts_terms(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    // One term needs no check: every row that matches holds it.
    let eligible = if terms.len() > 1 {
        let ids = meetings_with_every(conn, &terms)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        Some(ids)
    } else {
        None
    };
    let eligible_json = eligible.as_ref().map(serde_json::to_string).transpose()?;
    // Among those meetings, every place holding any of the words is ranked
    // and marked.
    let fts = terms.join(" OR ");
    let mut candidates: HashMap<i64, Candidate> = HashMap::new();
    let mut statement = conn.prepare(
        "SELECT rowid, highlight(meetings_fts, 0, char(1), char(2)),
                snippet(meetings_fts, 1, char(1), char(2), '…', 16),
                bm25(meetings_fts, 10.0, 1.0)
         FROM meetings_fts WHERE meetings_fts MATCH ?1",
    )?;
    let mut rows = statement.query(params![fts])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if eligible.as_ref().is_some_and(|ids| !ids.contains(&id)) {
            continue;
        }
        let marked_title: Option<String> = row.get(1)?;
        let marked_summary: Option<String> = row.get(2)?;
        let has_hit =
            |text: &Option<String>| text.as_deref().is_some_and(|t| t.contains(HIT_START));
        candidates.insert(
            id,
            Candidate {
                place: if has_hit(&marked_title) {
                    HitPlace::Title
                } else {
                    HitPlace::Summary
                },
                score: row.get(3)?,
                marked_summary: has_hit(&marked_summary).then_some(marked_summary).flatten(),
                marked_title,
                turns: Vec::new(),
            },
        );
    }
    // Ranking costs time for each matching turn. A query that matches more
    // turns than are read (a word in nearly every turn, two letters typed)
    // reads the newest instead, unranked: the meetings come newest first.
    // `?3`, when set, keeps the turns of the meetings holding every word.
    let broad: bool = conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM meeting_turns_fts
             JOIN meeting_turns t ON t.rowid = meeting_turns_fts.rowid
             WHERE meeting_turns_fts MATCH ?1
               AND (?3 IS NULL OR t.meeting_id IN (SELECT value FROM json_each(?3)))
             ORDER BY meeting_turns_fts.rowid DESC LIMIT 1 OFFSET ?2)",
        params![fts, MAX_TURN_HITS, eligible_json],
        |row| row.get(0),
    )?;
    let mut statement = conn.prepare(if broad {
        "SELECT t.meeting_id, t.seq, t.rowid, 0.0
         FROM meeting_turns_fts JOIN meeting_turns t ON t.rowid = meeting_turns_fts.rowid
         WHERE meeting_turns_fts MATCH ?1
           AND (?3 IS NULL OR t.meeting_id IN (SELECT value FROM json_each(?3)))
         ORDER BY meeting_turns_fts.rowid DESC LIMIT ?2"
    } else {
        "SELECT t.meeting_id, t.seq, t.rowid, bm25(meeting_turns_fts)
         FROM meeting_turns_fts JOIN meeting_turns t ON t.rowid = meeting_turns_fts.rowid
         WHERE meeting_turns_fts MATCH ?1
           AND (?3 IS NULL OR t.meeting_id IN (SELECT value FROM json_each(?3)))"
    })?;
    let mut rows = statement.query(params![fts, MAX_TURN_HITS, eligible_json])?;
    while let Some(row) = rows.next()? {
        let (id, seq, rowid, score): (i64, i64, i64, f64) =
            (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
        let candidate = candidates.entry(id).or_insert(Candidate {
            place: HitPlace::Transcript,
            score,
            marked_title: None,
            marked_summary: None,
            turns: Vec::new(),
        });
        if candidate.place == HitPlace::Transcript {
            candidate.score = candidate.score.min(score);
        }
        candidate.turns.push((seq, rowid));
        candidate.turns.sort_unstable();
        candidate.turns.truncate(MAX_SNIPPETS);
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<i64> = candidates.keys().copied().collect();
    let mut ranked = conn
        .prepare(
            "SELECT id, title, started_at_ms, duration_ms, status FROM meetings
             WHERE id IN (SELECT value FROM json_each(?1))",
        )?
        .query_map(params![serde_json::to_string(&ids)?], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .filter_map(|row| match row {
            Ok((id, title, started_at_ms, duration_ms, status)) => {
                let status = MeetingStatus::from_db(&status)?;
                let candidate = candidates.remove(&id)?;
                Some(Ok((
                    candidate,
                    id,
                    title,
                    started_at_ms,
                    duration_ms,
                    status,
                )))
            }
            Err(error) => Some(Err(error)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ranked.sort_by(|(a, _, _, a_start, ..), (b, _, _, b_start, ..)| {
        a.place
            .cmp(&b.place)
            .then(a.score.total_cmp(&b.score))
            .then(b_start.cmp(a_start))
    });
    ranked.truncate(limit);

    let mut turn_snippet = conn.prepare(
        "SELECT snippet(meeting_turns_fts, 0, char(1), char(2), '…', 12)
         FROM meeting_turns_fts WHERE meeting_turns_fts MATCH ?1 AND rowid = ?2",
    )?;
    let mut speaker_names =
        conn.prepare("SELECT key, display_name FROM meeting_speakers WHERE meeting_id = ?1")?;
    let mut hits = Vec::with_capacity(ranked.len());
    for (candidate, id, title, started_at_ms, duration_ms, status) in ranked {
        let mut snippets = Vec::new();
        if let Some(summary) = &candidate.marked_summary {
            let names: HashMap<String, String> = speaker_names
                .query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            snippets.push(Snippet::from_marked(&summary_line(summary, &names)));
        }
        for (_, rowid) in &candidate.turns {
            if snippets.len() == MAX_SNIPPETS {
                break;
            }
            let marked: String = turn_snippet.query_row(params![fts, rowid], |row| row.get(0))?;
            snippets.push(Snippet::from_marked(&marked));
        }
        hits.push(MeetingHit {
            id,
            title: candidate
                .marked_title
                .or(title)
                .map(|title| Snippet::from_marked(&title)),
            started_at_ms,
            duration_ms,
            status,
            snippets,
            first_turn_seq: candidate.turns.first().map(|(seq, _)| *seq),
        });
    }
    Ok(hits)
}

/// The meetings holding each of `terms` somewhere: in the title, the
/// summary or any turn.
fn meetings_with_every(conn: &Connection, terms: &[String]) -> Result<HashSet<i64>> {
    let mut in_title_or_summary =
        conn.prepare("SELECT rowid FROM meetings_fts WHERE meetings_fts MATCH ?1")?;
    let mut in_turns = conn.prepare(
        "SELECT DISTINCT t.meeting_id
         FROM meeting_turns_fts JOIN meeting_turns t ON t.rowid = meeting_turns_fts.rowid
         WHERE meeting_turns_fts MATCH ?1",
    )?;
    let mut every: Option<HashSet<i64>> = None;
    for term in terms {
        let mut holding = in_title_or_summary
            .query_map(params![term], |row| row.get(0))?
            .collect::<rusqlite::Result<HashSet<i64>>>()?;
        for id in in_turns.query_map(params![term], |row| row.get(0))? {
            holding.insert(id?);
        }
        if let Some(every) = &every {
            holding.retain(|id| every.contains(id));
        }
        let none_left = holding.is_empty();
        every = Some(holding);
        if none_left {
            break;
        }
    }
    Ok(every.unwrap_or_default())
}

/// A summary passage as one line of plain text: `[[key]]` markers become
/// the speakers' names and Markdown marks are dropped. Hits inside a
/// marker are dropped with it: names are not searched.
fn summary_line(marked: &str, names: &HashMap<String, String>) -> String {
    with_speaker_names(marked, names)
        .lines()
        .map(|line| {
            line.trim_start_matches(['#', '-', '*', '>', ' '])
                .trim_end()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace("**", "")
}

/// `marked` with each `[[key]]` replaced by that speaker's name, or by the
/// key itself for a speaker without one.
pub fn with_speaker_names(marked: &str, names: &HashMap<String, String>) -> String {
    let mut named = String::with_capacity(marked.len());
    let mut rest = marked;
    while let Some(start) = rest.find("[[") {
        let Some(length) = rest[start + 2..].find("]]") else {
            break;
        };
        let key: String = rest[start + 2..start + 2 + length]
            .chars()
            .filter(|c| *c != HIT_START && *c != HIT_END)
            .collect();
        named.push_str(&rest[..start]);
        named.push_str(names.get(&key).map_or(key.as_str(), String::as_str));
        rest = &rest[start + 2 + length + 2..];
    }
    named.push_str(rest);
    named
}

impl MeetingsStore {
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MeetingHit>> {
        search(&self.connection()?, query, limit)
    }
}
