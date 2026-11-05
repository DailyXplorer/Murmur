use serde_json::{json, Value};
use std::io::Cursor;
use std::path::Path;

use super::rpc::{serve, Server};
use super::sample::{seed_sample_data_dir, write_settings};
use super::tools::open_read_only;

#[test]
fn identifier_matches_tauri_conf() {
    let conf: Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(conf["identifier"], super::IDENTIFIER);
}

/// The environment of a test: the pairs given and nothing else.
fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
    let variables: std::collections::HashMap<String, std::ffi::OsString> = pairs
        .iter()
        .map(|(key, value)| (key.to_string(), std::ffi::OsString::from(value)))
        .collect();
    move |name| variables.get(name).cloned()
}

#[cfg(target_os = "windows")]
#[test]
fn the_data_dir_is_the_roaming_app_data_folder() {
    let pairs = [("APPDATA", r"C:\Users\Someone\AppData\Roaming")];

    assert_eq!(
        super::data_dir_from(environment(&pairs)),
        Some(std::path::PathBuf::from(r"C:\Users\Someone\AppData\Roaming").join(super::IDENTIFIER))
    );
    assert_eq!(super::data_dir_from(environment(&[])), None);
}

#[cfg(target_os = "macos")]
#[test]
fn the_data_dir_is_the_application_support_folder() {
    let pairs = [("HOME", "/home/someone")];

    assert_eq!(
        super::data_dir_from(environment(&pairs)),
        Some(std::path::PathBuf::from(
            "/home/someone/Library/Application Support/com.dailyxplorer.murmur"
        ))
    );
    assert_eq!(super::data_dir_from(environment(&[])), None);
}

#[test]
fn the_override_variable_names_the_data_dir() {
    let pairs = [
        ("MURMUR_MCP_DATA_DIR", "somewhere-else"),
        ("APPDATA", "ignored"),
        ("HOME", "ignored"),
    ];

    assert_eq!(
        super::data_dir_from(environment(&pairs)),
        Some(std::path::PathBuf::from("somewhere-else"))
    );
}

/// Sends `requests` to a server on `dir` and returns its replies, in order.
fn exchange(dir: &Path, requests: &[Value]) -> Vec<Value> {
    let input: String = requests
        .iter()
        .map(|request| format!("{request}\n"))
        .collect();
    let mut output = Vec::new();
    serve(
        &Server::new(dir.to_path_buf()),
        Cursor::new(input),
        &mut output,
    )
    .unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn request(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// The reply to one call of `tool`: its flag and its JSON text.
fn call(dir: &Path, tool: &str, arguments: Value) -> (bool, Value) {
    let reply = exchange(
        dir,
        &[request(
            1,
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        )],
    );
    let result = &reply[0]["result"];
    let text = result["content"][0]["text"].as_str().unwrap();
    let body = serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()));
    (result["isError"].as_bool().unwrap(), body)
}

fn seeded(enabled: bool, dictations: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    seed_sample_data_dir(dir.path(), enabled, dictations).unwrap();
    dir
}

fn tool_names(dir: &Path) -> Vec<String> {
    let reply = exchange(dir, &[request(1, "tools/list", json!({}))]);
    reply[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn mcp_initialize_and_list_tools() {
    let dir = seeded(true, false);
    let replies = exchange(
        dir.path(),
        &[
            request(1, "initialize", json!({"protocolVersion": "2025-06-18"})),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            request(2, "ping", json!({})),
            request(3, "initialize", json!({"protocolVersion": "1999-01-01"})),
            request(4, "tools/list", json!({})),
        ],
    );

    // The notification gets no reply.
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "murmur");
    assert_eq!(
        replies[0]["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(replies[1]["result"], json!({}));
    assert_eq!(replies[2]["result"]["protocolVersion"], "2025-11-25");
    let tools = replies[3]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[test]
fn malformed_messages_get_json_rpc_errors() {
    let dir = seeded(true, false);
    let mut output = Vec::new();
    let input = format!(
        "not json\n{}\n{}\n[1]\n",
        request(1, "resources/list", json!({})),
        request(
            2,
            "tools/call",
            json!({"name": "delete_meeting", "arguments": {}})
        ),
    );
    serve(
        &Server::new(dir.path().to_path_buf()),
        Cursor::new(input),
        &mut output,
    )
    .unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert_eq!(replies[0]["error"]["code"], -32700);
    assert_eq!(replies[1]["error"]["code"], -32601);
    assert_eq!(replies[2]["error"]["code"], -32602);
    assert_eq!(replies[3]["error"]["code"], -32600);
}

#[test]
fn every_listed_tool_is_read_only() {
    let dir = seeded(true, true);

    let reply = exchange(dir.path(), &[request(1, "tools/list", json!({}))]);

    let tools = reply[0]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 5);
    for tool in tools {
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[test]
fn mcp_refuses_tools_when_disabled() {
    let dir = seeded(false, true);

    assert!(tool_names(dir.path()).is_empty());
    for (tool, arguments) in [
        ("list_meetings", json!({})),
        ("search", json!({"query": "roadmap"})),
        ("get_transcript", json!({"id": 1})),
        ("get_summary", json!({"id": 1})),
        ("recent_dictations", json!({})),
    ] {
        let (is_error, body) = call(dir.path(), tool, arguments);
        assert!(is_error, "{tool}");
        let message = body.as_str().unwrap();
        assert!(message.contains("MCP access is off"), "{tool}: {message}");
        assert!(!message.contains("Roadmap"), "{tool}");
    }
}

#[test]
fn turning_the_switch_off_applies_to_a_running_server() {
    let dir = seeded(true, false);
    assert!(!call(dir.path(), "list_meetings", json!({})).0);

    write_settings(dir.path(), false, false).unwrap();

    assert!(call(dir.path(), "list_meetings", json!({})).0);
    assert!(tool_names(dir.path()).is_empty());
}

#[test]
fn mcp_hides_dictations_unless_included() {
    let dir = seeded(true, false);

    assert_eq!(
        tool_names(dir.path()),
        ["list_meetings", "search", "get_transcript", "get_summary"]
    );
    let (is_error, body) = call(dir.path(), "recent_dictations", json!({}));
    assert!(is_error);
    assert!(body.as_str().unwrap().contains("Include dictation history"));
    let (is_error, _) = call(
        dir.path(),
        "search",
        json!({"query": "invoice", "scope": "dictations"}),
    );
    assert!(is_error);
    // "All" then means meetings only, and the dictation is not found.
    let (is_error, body) = call(dir.path(), "search", json!({"query": "invoice"}));
    assert!(!is_error);
    assert!(body.get("dictations").is_none());
    assert_eq!(body["meetings"], json!([]));

    write_settings(dir.path(), true, true).unwrap();

    assert_eq!(tool_names(dir.path()).len(), 5);
    assert!(!call(dir.path(), "recent_dictations", json!({})).0);
}

#[test]
fn mcp_opens_db_read_only() {
    let dir = seeded(true, true);
    for file in ["meetings.db", "history.db"] {
        let conn = open_read_only(&dir.path().join(file)).unwrap();
        for write in [
            "DELETE FROM meetings",
            "DELETE FROM transcription_history",
            "CREATE TABLE intruder (x)",
            "PRAGMA user_version = 99",
        ] {
            let _ = conn.execute_batch(write);
        }
        // Not even a connection that switches `query_only` off can write.
        conn.pragma_update(None, "query_only", false).unwrap();
        let error = conn
            .execute_batch(if file == "meetings.db" {
                "UPDATE meetings SET title = 'changed'"
            } else {
                "UPDATE transcription_history SET title = 'changed'"
            })
            .unwrap_err();
        assert!(error.to_string().contains("readonly"), "{file}: {error}");
    }
    // Everything is still there.
    let (is_error, body) = call(dir.path(), "list_meetings", json!({}));
    assert!(!is_error);
    assert_eq!(body["meetings"].as_array().unwrap().len(), 3);
}

#[test]
fn list_meetings_pages_newest_first() {
    let dir = seeded(true, false);

    let (_, first) = call(dir.path(), "list_meetings", json!({"limit": 2}));

    let ids: Vec<i64> = first["meetings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|meeting| meeting["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [3, 2]);
    assert_eq!(first["next_before_id"], 2);
    assert_eq!(first["meetings"][0]["status"], "processing");
    assert_eq!(first["meetings"][0]["title"], Value::Null);
    assert_eq!(first["meetings"][1]["title"], "Réunion budget");
    assert_eq!(first["meetings"][1]["started_at"], "2026-03-09T15:30:00Z");

    let (_, second) = call(
        dir.path(),
        "list_meetings",
        json!({"limit": 2, "before_id": 2}),
    );
    assert_eq!(second["meetings"].as_array().unwrap().len(), 1);
    assert_eq!(second["meetings"][0]["title"], "Roadmap review");
    assert_eq!(second["meetings"][0]["duration_ms"], 1_800_000);
    assert_eq!(second["next_before_id"], Value::Null);
}

#[test]
fn search_finds_meetings_and_dictations() {
    let dir = seeded(true, true);

    let (is_error, body) = call(dir.path(), "search", json!({"query": "roadmap"}));
    assert!(!is_error);
    assert_eq!(body["meetings"].as_array().unwrap().len(), 1);
    assert_eq!(body["meetings"][0]["id"], 1);
    assert_eq!(body["meetings"][0]["title"], "Roadmap review");
    assert!(body["meetings"][0]["snippets"][0]
        .as_str()
        .unwrap()
        .contains("roadmap"));
    assert_eq!(body["dictations"], json!([]));

    // Accents fold, and a summary names its people.
    let (_, body) = call(dir.path(), "search", json!({"query": "reunion budget"}));
    assert_eq!(body["meetings"][0]["id"], 2);
    let (_, body) = call(dir.path(), "search", json!({"query": "checklist"}));
    assert_eq!(body["meetings"][0]["first_match_seq"], 1);
    assert!(body["meetings"][0]["snippets"][0]
        .as_str()
        .unwrap()
        .contains("Product lead owns"));

    let (_, body) = call(
        dir.path(),
        "search",
        json!({"query": "invoice", "scope": "dictations"}),
    );
    assert!(body.get("meetings").is_none());
    assert_eq!(body["dictations"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["dictations"][0]["text"],
        "Remind me to send the invoice on Friday."
    );
    assert!(!body.to_string().contains("private-recording"));
    assert!(!body.to_string().contains("raw uncleaned"));

    let (is_error, _) = call(dir.path(), "search", json!({"query": 5}));
    assert!(is_error);
    let (_, body) = call(dir.path(), "search", json!({"query": "\"NEAR(\" - *"}));
    assert_eq!(body["meetings"], json!([]));
}

#[test]
fn search_finds_dictations_and_returns_their_text_only() {
    let dir = seeded(true, true);

    let (is_error, body) = call(
        dir.path(),
        "search",
        json!({"query": "invoice", "scope": "dictations"}),
    );

    assert!(!is_error);
    assert_eq!(body["dictations"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["dictations"][0]["text"],
        "Remind me to send the invoice on Friday."
    );
    assert_eq!(body["dictations"][0]["at"], "2026-03-08T20:00:00Z");
    assert!(!body.to_string().contains("private-recording"));
    assert!(!body.to_string().contains("raw uncleaned"));
    // A prefix matches and FTS5 syntax is only text.
    let (_, body) = call(
        dir.path(),
        "search",
        json!({"query": "plumb", "scope": "dictations"}),
    );
    assert_eq!(body["dictations"].as_array().unwrap().len(), 1);
    let (is_error, body) = call(
        dir.path(),
        "search",
        json!({"query": "\"NEAR(\" - *", "scope": "dictations"}),
    );
    assert!(!is_error);
    assert_eq!(body["dictations"], json!([]));
    assert!(
        call(
            dir.path(),
            "search",
            json!({"query": 5, "scope": "dictations"})
        )
        .0
    );
}

#[test]
fn get_transcript_names_speakers_and_pages() {
    let dir = seeded(true, false);

    let (is_error, page) = call(dir.path(), "get_transcript", json!({"id": 1, "limit": 2}));

    assert!(!is_error);
    assert_eq!(page["title"], "Roadmap review");
    assert_eq!(page["total_turns"], 3);
    assert_eq!(page["next_from_seq"], 2);
    let turns = page["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["speaker"], "Me");
    assert_eq!(turns[0].get("low_confidence"), None);
    assert_eq!(turns[1]["speaker"], "Product lead");
    assert_eq!(turns[1]["low_confidence"], true);
    assert_eq!(turns[1]["start_ms"], 6000);
    assert_eq!(turns[1]["text"], "The launch checklist is nearly done.");

    let (_, last) = call(
        dir.path(),
        "get_transcript",
        json!({"id": 1, "from_seq": 2, "limit": 2}),
    );
    assert_eq!(last["turns"].as_array().unwrap().len(), 1);
    assert_eq!(last["next_from_seq"], Value::Null);

    let (is_error, body) = call(dir.path(), "get_transcript", json!({"id": 99}));
    assert!(is_error);
    assert_eq!(body, "There is no meeting 99.");
    assert!(call(dir.path(), "get_transcript", json!({})).0);
}

#[test]
fn get_summary_names_people() {
    let dir = seeded(true, false);

    let (_, summary) = call(dir.path(), "get_summary", json!({"id": 1}));

    assert_eq!(summary["title"], "Roadmap review");
    assert_eq!(summary["summary_status"], "ready");
    let text = summary["summary"].as_str().unwrap();
    assert!(
        text.contains("Product lead owns the launch checklist."),
        "{text}"
    );
    assert!(!text.contains("[["));

    let (_, pending) = call(dir.path(), "get_summary", json!({"id": 3}));
    assert_eq!(pending["summary"], Value::Null);
    assert_eq!(pending["summary_status"], "pending");
}

#[test]
fn recent_dictations_return_text_and_time_only() {
    let dir = seeded(true, true);

    let (is_error, body) = call(dir.path(), "recent_dictations", json!({}));

    assert!(!is_error);
    let dictations = body["dictations"].as_array().unwrap();
    assert_eq!(dictations.len(), 2);
    assert_eq!(dictations[0]["text"], "Call the plumber tomorrow morning.");
    assert_eq!(dictations[0]["at"], "2026-03-08T20:01:40Z");
    let mut keys: Vec<_> = dictations[0].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["at", "id", "text"]);
    assert!(!body.to_string().contains("private-recording"));

    let (_, newer) = call(
        dir.path(),
        "recent_dictations",
        json!({"since_ms": 1_773_000_050_000_i64}),
    );
    assert_eq!(newer["dictations"].as_array().unwrap().len(), 1);
    let (_, one) = call(dir.path(), "recent_dictations", json!({"limit": 1}));
    assert_eq!(one["dictations"].as_array().unwrap().len(), 1);
}

#[test]
fn a_library_the_app_has_not_updated_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), true, false).unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("meetings.db")).unwrap();
    conn.execute_batch("CREATE TABLE meetings (id INTEGER PRIMARY KEY)")
        .unwrap();
    drop(conn);

    let (is_error, body) = call(dir.path(), "list_meetings", json!({}));

    assert!(is_error);
    assert!(body.as_str().unwrap().contains("Open Murmur once"));
    // Reading never migrated it.
    let conn = open_read_only(&dir.path().join("meetings.db")).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 0);
}

#[test]
fn without_a_library_there_is_nothing_to_list() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), true, true).unwrap();

    assert_eq!(
        call(dir.path(), "list_meetings", json!({})).1["meetings"],
        json!([])
    );
    assert_eq!(
        call(dir.path(), "recent_dictations", json!({})).1["dictations"],
        json!([])
    );
    assert!(call(dir.path(), "get_summary", json!({"id": 1})).0);
}
