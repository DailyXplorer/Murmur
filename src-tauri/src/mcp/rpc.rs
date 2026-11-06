//! JSON-RPC 2.0 over stdio, one message per line, for the handshake-based
//! revisions of the Model Context Protocol.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use super::access::Access;
use super::tools;

/// Protocol revisions this server speaks, newest first. A client asking for
/// another one is answered with the first.
const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// What the AI app is told about the server.
const INSTRUCTIONS: &str =
    "Read-only access to the user's Murmur meetings (and, if they allowed it, dictations). \
Use list_meetings or search to find a meeting, then get_summary or get_transcript. \
Nothing here can change anything.";

pub struct Server {
    pub data_dir: PathBuf,
}

impl Server {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// Answers each line of `input` until it ends.
pub fn serve(server: &Server, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_line(server, &line) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// The reply to one message, or `None` for a notification.
fn handle_line(server: &Server, line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(_) => return Some(failure(Value::Null, PARSE_ERROR, "Parse error")),
    };
    if !message.is_object() {
        return Some(failure(Value::Null, INVALID_REQUEST, "Invalid request"));
    }
    let Some(method) = message["method"].as_str() else {
        // A response to a request we never send is ignored; anything else
        // with an id is not a JSON-RPC request.
        let is_response = message.get("result").is_some() || message.get("error").is_some();
        return if message.get("id").is_some() && !is_response {
            Some(failure(
                message["id"].clone(),
                INVALID_REQUEST,
                "Invalid request",
            ))
        } else {
            None
        };
    };
    // A request without an id is a notification: it gets no reply.
    let id = message.get("id")?.clone();
    let reply = match dispatch(server, method, &message["params"]) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => failure(id, error.code, &error.message),
    };
    Some(reply)
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn dispatch(server: &Server, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        // The access is read again for each request: see `access`.
        "tools/list" => Ok(tools::list(Access::read(&server.data_dir))),
        "tools/call" => {
            let name = params["name"].as_str().ok_or_else(|| RpcError {
                code: INVALID_PARAMS,
                message: "Missing tool name".to_string(),
            })?;
            tools::call(server, name, &params["arguments"])
        }
        _ => Err(RpcError {
            code: METHOD_NOT_FOUND,
            message: format!("Method not found: {method}"),
        }),
    }
}

fn initialize(params: &Value) -> Value {
    let version = params["protocolVersion"]
        .as_str()
        .and_then(|asked| PROTOCOL_VERSIONS.iter().find(|known| **known == asked))
        .unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "murmur", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}
