//! The tools: five where meetings exist (macOS), `search` over dictations and
//! `recent_dictations` where they do not (Windows). Each one reads through a
//! connection opened read-only and returns JSON text. None takes a path, and
//! none returns audio, audio paths, window names, field excerpts, settings or
//! account data.

use anyhow::Error;
use chrono::{DateTime, SecondsFormat};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use super::access::Access;
use super::rpc::{RpcError, Server, INVALID_PARAMS};
use super::MEETINGS_EXIST;
use crate::managers::history::{self, HistoryManager};
use crate::meetings::store::{self, list_page, with_speaker_names};
use crate::search::search_failed;

/// How long a read waits for the app, which may be writing.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

const OFF_MESSAGE: &str = "Murmur MCP access is off. Turn it on in Murmur > Settings > Advanced.";
const DICTATIONS_OFF_MESSAGE: &str = "Dictation history is not shared. Turn on \"Include dictation history\" in Murmur > Settings > Advanced.";
const UPDATE_MESSAGE: &str =
    "Murmur's data is not ready for this version. Open Murmur once to finish updating.";
const READ_FAILED_MESSAGE: &str = "Murmur could not read its data. Try again.";
const MEETINGS_SEARCH: &str = "Find meetings by the words in their title, summary or transcript. Every word must appear somewhere in the meeting; a word matches as a prefix; \"double quotes\" match a phrase.";
const DICTATIONS_SEARCH: &str = "Find dictations by the words in their text. Every word must appear in the dictation; a word matches as a prefix; \"double quotes\" match a phrase.";
const SCOPE_MESSAGE: &str = if MEETINGS_EXIST {
    "scope must be meetings, dictations or all"
} else {
    "scope must be dictations"
};

const MEETINGS_FILE: &str = "meetings.db";
const HISTORY_FILE: &str = "history.db";

const MAX_LIST: usize = 50;
const MAX_SEARCH: usize = 20;
const MAX_TURNS: usize = 400;
const MAX_DICTATIONS: usize = 50;

/// Opens a database that can only be read: SQLite refuses every write, and
/// `query_only` refuses them again at the statement level.
pub fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "query_only", true)?;
    Ok(conn)
}

/// The tools the user's settings allow, as `tools/list` answers.
pub fn list(access: Access) -> Value {
    let meetings = access.meetings && MEETINGS_EXIST;
    let mut tools = Vec::new();
    if meetings {
        tools.push(tool(
            "list_meetings",
            "List the user's recorded meetings, newest first: id, title, start time, duration and status.",
            json!({
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIST, "description": "How many meetings (default 20)."},
                "before_id": {"type": "integer", "description": "Only meetings with a smaller id: pass next_before_id from the previous page."}
            }),
            &[],
        ));
    }
    if meetings || access.dictations {
        tools.push(search_tool(meetings, access.dictations));
    }
    if meetings {
        tools.push(tool(
            "get_transcript",
            "Read a meeting's transcript turn by turn, with who spoke and when. Long meetings come in pages: pass next_from_seq to read on.",
            json!({
                "id": {"type": "integer", "description": "The meeting id."},
                "from_seq": {"type": "integer", "minimum": 0, "description": "The first turn to read (default 0)."},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_TURNS, "description": "How many turns (default 200)."}
            }),
            &["id"],
        ));
        tools.push(tool(
            "get_summary",
            "Read a meeting's title and summary (Markdown, with people's names).",
            json!({"id": {"type": "integer", "description": "The meeting id."}}),
            &["id"],
        ));
    }
    if access.dictations {
        tools.push(tool(
            "recent_dictations",
            "The user's latest dictations, newest first: the text they dictated and when.",
            json!({
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_DICTATIONS, "description": "How many dictations (default 20)."},
                "since_ms": {"type": "integer", "description": "Only dictations after this time, in milliseconds since 1970."}
            }),
            &[],
        ));
    }
    json!({"tools": tools})
}

/// `search`, worded for what the user let it look in.
fn search_tool(meetings: bool, dictations: bool) -> Value {
    let (description, scopes, default_scope) = match (meetings, dictations) {
        (true, true) => (
            MEETINGS_SEARCH,
            json!(["meetings", "dictations", "all"]),
            "all",
        ),
        (true, false) => (MEETINGS_SEARCH, json!(["meetings"]), "meetings"),
        (false, _) => (DICTATIONS_SEARCH, json!(["dictations"]), "dictations"),
    };
    tool(
        "search",
        description,
        json!({
            "query": {"type": "string", "description": "The words to look for."},
            "scope": {"type": "string", "enum": scopes, "description": format!("Where to look (default {default_scope}).")},
            "limit": {"type": "integer", "minimum": 1, "maximum": MAX_SEARCH, "description": "How many results (default 10)."}
        }),
        &["query"],
    )
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

/// Runs a tool. A tool that cannot answer returns a result flagged as an
/// error, which the AI app shows its model; only an unknown tool is a
/// protocol error.
pub fn call(server: &Server, name: &str, arguments: &Value) -> Result<Value, RpcError> {
    let access = Access::read(&server.data_dir);
    if !access.meetings {
        return Ok(failed_result(OFF_MESSAGE));
    }
    let dir = server.data_dir.as_path();
    let args = Args(arguments);
    let outcome = match name {
        "list_meetings" if MEETINGS_EXIST => list_meetings(dir, &args),
        "search" => search(dir, access, &args),
        "get_transcript" if MEETINGS_EXIST => get_transcript(dir, &args),
        "get_summary" if MEETINGS_EXIST => get_summary(dir, &args),
        "recent_dictations" => recent_dictations(dir, access, &args),
        _ => {
            return Err(RpcError {
                code: INVALID_PARAMS,
                message: format!("Unknown tool: {name}"),
            })
        }
    };
    Ok(match outcome {
        Ok(value) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "isError": false
        }),
        Err(message) => failed_result(&message),
    })
}

fn failed_result(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

/// The arguments of a call, read leniently: a limit out of range is brought
/// into it, a value of the wrong type is refused.
struct Args<'a>(&'a Value);

impl Args<'_> {
    fn text(&self, key: &str) -> Result<Option<&str>, String> {
        match &self.0[key] {
            Value::Null => Ok(None),
            Value::String(text) => Ok(Some(text)),
            _ => Err(format!("{key} must be a string")),
        }
    }

    fn integer(&self, key: &str) -> Result<Option<i64>, String> {
        match &self.0[key] {
            Value::Null => Ok(None),
            value => value
                .as_i64()
                .map(Some)
                .ok_or_else(|| format!("{key} must be an integer")),
        }
    }

    fn required_id(&self) -> Result<i64, String> {
        self.integer("id")?
            .ok_or_else(|| "id is required".to_string())
    }

    fn count(&self, key: &str, default: usize, max: usize) -> Result<usize, String> {
        Ok(self
            .integer(key)?
            .map_or(default, |asked| asked.clamp(1, max as i64) as usize))
    }
}

fn failed(error: impl Into<Error>) -> String {
    search_failed("MCP", &error.into());
    READ_FAILED_MESSAGE.to_string()
}

fn iso(ms: i64) -> Option<String> {
    DateTime::from_timestamp_millis(ms).map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true))
}

/// The database `file` of the app, if the app ever created it. A library
/// the app has not migrated to this version is refused: the server never
/// migrates.
fn open_database(
    dir: &Path,
    file: &str,
    migrations: &rusqlite_migration::Migrations<'_>,
) -> Result<Option<Connection>, String> {
    let path = dir.join(file);
    if !path.exists() {
        return Ok(None);
    }
    let conn = open_read_only(&path).map_err(failed)?;
    if !matches!(migrations.pending_migrations(&conn), Ok(0)) {
        return Err(UPDATE_MESSAGE.to_string());
    }
    Ok(Some(conn))
}

fn meetings_db(dir: &Path) -> Result<Option<Connection>, String> {
    open_database(dir, MEETINGS_FILE, &store::migrations())
}

fn history_db(dir: &Path) -> Result<Option<Connection>, String> {
    open_database(dir, HISTORY_FILE, &history::migrations())
}

/// The names the meeting's speakers go by now, by speaker key.
fn speaker_names(conn: &Connection, meeting: i64) -> Result<HashMap<String, String>, String> {
    let mut statement = conn
        .prepare("SELECT key, display_name FROM meeting_speakers WHERE meeting_id = ?1")
        .map_err(failed)?;
    let names = statement
        .query_map(params![meeting], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(failed)?
        .collect::<rusqlite::Result<HashMap<String, String>>>()
        .map_err(failed)?;
    Ok(names)
}

fn list_meetings(dir: &Path, args: &Args) -> Result<Value, String> {
    let limit = args.count("limit", 20, MAX_LIST)?;
    let before = args.integer("before_id")?;
    let Some(conn) = meetings_db(dir)? else {
        return Ok(json!({"meetings": [], "next_before_id": null}));
    };
    let page = list_page(&conn, before, limit).map_err(failed)?;
    let mut meetings = Vec::with_capacity(page.meetings.len());
    for meeting in &page.meetings {
        let names = speaker_names(&conn, meeting.id)?;
        meetings.push(json!({
            "id": meeting.id,
            "title": meeting.title.as_deref().map(|title| with_speaker_names(title, &names)),
            "started_at": iso(meeting.started_at_ms),
            "duration_ms": meeting.duration_ms,
            "status": meeting.status,
        }));
    }
    let next = page.has_more.then(|| page.meetings.last().map(|m| m.id));
    Ok(json!({"meetings": meetings, "next_before_id": next.flatten()}))
}

fn search(dir: &Path, access: Access, args: &Args) -> Result<Value, String> {
    let query = args
        .text("query")?
        .ok_or_else(|| "query is required".to_string())?;
    let limit = args.count("limit", 10, MAX_SEARCH)?;
    let scope = args
        .text("scope")?
        .unwrap_or(if MEETINGS_EXIST { "all" } else { "dictations" });
    let (in_meetings, in_dictations) = match scope {
        "meetings" if MEETINGS_EXIST => (true, false),
        "dictations" => (false, true),
        "all" if MEETINGS_EXIST => (true, access.dictations),
        _ => return Err(SCOPE_MESSAGE.to_string()),
    };
    if in_dictations && !access.dictations {
        return Err(DICTATIONS_OFF_MESSAGE.to_string());
    }

    let mut result = json!({});
    if in_meetings {
        let mut meetings = Vec::new();
        if let Some(conn) = meetings_db(dir)? {
            for hit in store::search(&conn, query, limit).map_err(failed)? {
                let names = speaker_names(&conn, hit.id)?;
                meetings.push(json!({
                    "id": hit.id,
                    "title": hit.title.map(|title| with_speaker_names(&title.text, &names)),
                    "started_at": iso(hit.started_at_ms),
                    "snippets": hit.snippets.iter().map(|snippet| &snippet.text).collect::<Vec<_>>(),
                    "first_match_seq": hit.first_turn_seq,
                }));
            }
        }
        result["meetings"] = Value::Array(meetings);
    }
    if in_dictations {
        let mut dictations = Vec::new();
        if let Some(conn) = history_db(dir)? {
            for hit in HistoryManager::search_with_conn(&conn, query, limit).map_err(failed)? {
                dictations.push(json!({
                    "id": hit.entry.id,
                    "at": iso(hit.entry.timestamp.saturating_mul(1000)),
                    "text": hit.entry.transcription_text,
                }));
            }
        }
        result["dictations"] = Value::Array(dictations);
    }
    Ok(result)
}

fn get_transcript(dir: &Path, args: &Args) -> Result<Value, String> {
    let id = args.required_id()?;
    let from_seq = args.integer("from_seq")?.unwrap_or(0).max(0);
    let limit = args.count("limit", 200, MAX_TURNS)?;
    let conn = meetings_db(dir)?.ok_or_else(|| not_found(id))?;
    let title: Option<String> = conn
        .query_row(
            "SELECT title FROM meetings WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(failed)?
        .ok_or_else(|| not_found(id))?;
    let names = speaker_names(&conn, id)?;
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM meeting_turns WHERE meeting_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(failed)?;
    // One turn more than asked for says whether another page follows.
    let mut statement = conn
        .prepare(
            "SELECT seq, speaker_key, start_ms, end_ms, text, low_confidence FROM meeting_turns
             WHERE meeting_id = ?1 AND seq >= ?2 ORDER BY seq LIMIT ?3",
        )
        .map_err(failed)?;
    let mut turns = statement
        .query_map(params![id, from_seq, limit as i64 + 1], |row| {
            let seq: i64 = row.get(0)?;
            let key: String = row.get(1)?;
            let mut turn = json!({
                "seq": seq,
                "speaker": names.get(&key).unwrap_or(&key),
                "start_ms": row.get::<_, i64>(2)?,
                "end_ms": row.get::<_, i64>(3)?,
                "text": row.get::<_, String>(4)?,
            });
            if row.get::<_, bool>(5)? {
                turn["low_confidence"] = Value::Bool(true);
            }
            Ok((seq, turn))
        })
        .map_err(failed)?
        .collect::<rusqlite::Result<Vec<(i64, Value)>>>()
        .map_err(failed)?;
    let next_from_seq = (turns.len() > limit).then(|| turns[limit].0);
    turns.truncate(limit);
    Ok(json!({
        "id": id,
        "title": title.as_deref().map(|title| with_speaker_names(title, &names)),
        "total_turns": total,
        "turns": turns.into_iter().map(|(_, turn)| turn).collect::<Vec<_>>(),
        "next_from_seq": next_from_seq,
    }))
}

fn get_summary(dir: &Path, args: &Args) -> Result<Value, String> {
    let id = args.required_id()?;
    let conn = meetings_db(dir)?.ok_or_else(|| not_found(id))?;
    let (title, summary, status): (Option<String>, Option<String>, String) = conn
        .query_row(
            "SELECT title, summary_md, summary_status FROM meetings WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(failed)?
        .ok_or_else(|| not_found(id))?;
    let names = speaker_names(&conn, id)?;
    Ok(json!({
        "id": id,
        "title": title.as_deref().map(|title| with_speaker_names(title, &names)),
        "summary": summary.as_deref().map(|summary| with_speaker_names(summary, &names)),
        "summary_status": status,
    }))
}

fn not_found(id: i64) -> String {
    format!("There is no meeting {id}.")
}

fn recent_dictations(dir: &Path, access: Access, args: &Args) -> Result<Value, String> {
    if !access.dictations {
        return Err(DICTATIONS_OFF_MESSAGE.to_string());
    }
    let limit = args.count("limit", 20, MAX_DICTATIONS)?;
    let since = args.integer("since_ms")?.unwrap_or(0).div_euclid(1000);
    let Some(conn) = history_db(dir)? else {
        return Ok(json!({"dictations": []}));
    };
    let mut statement = conn
        .prepare(
            "SELECT id, timestamp, transcription_text FROM transcription_history
             WHERE transcription_text != '' AND timestamp > ?1
             ORDER BY timestamp DESC, id DESC LIMIT ?2",
        )
        .map_err(failed)?;
    let dictations = statement
        .query_map(params![since, limit as i64], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "at": iso(row.get::<_, i64>(1)?.saturating_mul(1000)),
                "text": row.get::<_, String>(2)?,
            }))
        })
        .map_err(failed)?
        .collect::<rusqlite::Result<Vec<Value>>>()
        .map_err(failed)?;
    Ok(json!({"dictations": dictations}))
}
