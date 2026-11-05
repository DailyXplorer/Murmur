//! Runs the built binary as `murmur mcp` on an invented data directory and
//! talks to it the way an AI app does: JSON-RPC lines over stdin and stdout.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Whether the app records meetings, and so offers the meeting tools (macOS).
const MEETINGS_EXIST: bool = cfg!(target_os = "macos");

/// How long any wait on the server may take: a slow runner is allowed, a hang
/// is not (nextest ends a test after two minutes).
const WAIT: Duration = Duration::from_secs(30);

/// Kills the server if a test panics before it ends it.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Session {
    child: KillOnDrop,
    stdin: ChildStdin,
    lines: Receiver<std::io::Result<String>>,
    next_id: i64,
}

impl Session {
    fn start(data_dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_murmur"))
            .arg("mcp")
            .env("MURMUR_MCP_DATA_DIR", data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the murmur binary starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        // A blocking read cannot time out, so a thread reads and the test
        // waits on its channel.
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child: KillOnDrop(child),
            stdin,
            lines,
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        let line = self
            .lines
            .recv_timeout(WAIT)
            .expect("a reply within the wait, before the server closed stdout")
            .unwrap();
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("a JSON line, got {line:?}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let reply = self.receive();
        assert_eq!(reply["id"], id);
        reply
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let reply = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &reply["result"];
        assert_eq!(result["isError"], false, "{tool}: {reply}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn initialize(&mut self) -> Value {
        let reply = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }),
        );
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        reply
    }

    /// Closes stdin, as an AI app does on exit, and waits for the process.
    fn finish(mut self) -> std::process::ExitStatus {
        drop(self.stdin);
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the server outlived its closed stdin"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn seeded(enabled: bool, dictations: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    murmur::mcp::seed_sample_data_dir(dir.path(), enabled, dictations).unwrap();
    dir
}

#[test]
fn every_tool_answers_over_stdio() {
    let dir = seeded(true, true);
    let mut session = Session::start(dir.path());

    let initialized = session.initialize();
    assert_eq!(initialized["result"]["serverInfo"]["name"], "murmur");
    let listed = session.request("tools/list", json!({}));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    if MEETINGS_EXIST {
        assert_eq!(
            names,
            [
                "list_meetings",
                "search",
                "get_transcript",
                "get_summary",
                "recent_dictations"
            ]
        );
    } else {
        assert_eq!(names, ["search", "recent_dictations"]);
    }

    if MEETINGS_EXIST {
        let meetings = session.call("list_meetings", json!({"limit": 5}));
        assert_eq!(meetings["meetings"].as_array().unwrap().len(), 3);
        assert_eq!(meetings["meetings"][2]["title"], "Roadmap review");

        let found = session.call("search", json!({"query": "roadmap budget"}));
        assert_eq!(found["meetings"][0]["id"], 1);
        assert_eq!(found["dictations"], json!([]));

        let transcript = session.call("get_transcript", json!({"id": 1}));
        assert_eq!(transcript["turns"].as_array().unwrap().len(), 3);
        assert_eq!(transcript["turns"][1]["speaker"], "Product lead");

        let summary = session.call("get_summary", json!({"id": 1}));
        assert!(summary["summary"]
            .as_str()
            .unwrap()
            .contains("Product lead owns the launch checklist."));
    }
    let found = session.call("search", json!({"query": "plumber"}));
    assert_eq!(
        found["dictations"][0]["text"],
        "Call the plumber tomorrow morning."
    );

    let dictations = session.call("recent_dictations", json!({"limit": 1}));
    assert_eq!(dictations["dictations"].as_array().unwrap().len(), 1);

    assert!(session.finish().success());
}

#[test]
fn a_server_with_the_switch_off_answers_nothing() {
    let dir = seeded(false, false);
    let mut session = Session::start(dir.path());

    session.initialize();
    let listed = session.request("tools/list", json!({}));
    assert_eq!(listed["result"]["tools"], json!([]));
    let called = session.request(
        "tools/call",
        json!({"name": "recent_dictations", "arguments": {}}),
    );
    assert_eq!(called["result"]["isError"], true);
    assert!(called["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("access is off"));

    assert!(session.finish().success());
}

/// The measurement searches a large library of meetings.
#[cfg(target_os = "macos")]
mod timing {
    use super::*;

    /// Words with a Zipf-like spread, from a fixed seed.
    struct Words {
        state: u64,
        vocabulary: Vec<String>,
    }

    impl Words {
        fn new() -> Self {
            const SYLLABLES: [&str; 12] = [
                "ba", "ko", "ri", "ten", "mo", "la", "vi", "sun", "de", "pra", "lo", "fe",
            ];
            let vocabulary = (0..20_000usize)
                .map(|index| {
                    let mut word = String::new();
                    let mut rest = index;
                    loop {
                        word.push_str(SYLLABLES[rest % SYLLABLES.len()]);
                        rest /= SYLLABLES.len();
                        if rest == 0 {
                            break;
                        }
                    }
                    word
                })
                .collect();
            Self {
                state: 0x2545_F491_4F6C_DD1D,
                vocabulary,
            }
        }

        fn word(&mut self) -> String {
            self.state ^= self.state << 13;
            self.state ^= self.state >> 7;
            self.state ^= self.state << 17;
            let unit = (self.state >> 11) as f64 / (1u64 << 53) as f64;
            // Squaring skews toward the first words, as speech does.
            let index = (unit * unit * self.vocabulary.len() as f64) as usize;
            self.vocabulary[index].clone()
        }

        fn sentence(&mut self, words: usize) -> String {
            (0..words)
                .map(|_| self.word())
                .collect::<Vec<_>>()
                .join(" ")
        }
    }

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    /// `cargo test --release --test mcp_stdio mcp_timing -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement: prints timings"]
    fn mcp_timing() {
        const MEETINGS: i64 = 300;
        const TURNS: i64 = 450;
        let dir = seeded(true, false);
        let mut conn = rusqlite::Connection::open(dir.path().join("meetings.db")).unwrap();
        let mut words = Words::new();
        let tx = conn.transaction().unwrap();
        for id in 100..100 + MEETINGS {
            tx.execute(
            "INSERT INTO meetings (id, started_at_ms, duration_ms, capture_mode, status, title, summary_md, summary_status)
             VALUES (?1, ?2, 3600000, 'both', 'ready', ?3, ?4, 'ready')",
            rusqlite::params![id, id * 86_400_000, words.sentence(4), words.sentence(120)],
        )
        .unwrap();
            for seq in 0..TURNS {
                tx.execute(
                "INSERT INTO meeting_turns (meeting_id, seq, speaker_key, start_ms, end_ms, text)
                 VALUES (?1, ?2, 'me', ?3, ?3 + 8000, ?4)",
                rusqlite::params![id, seq, seq * 8_000, words.sentence(20)],
            )
            .unwrap();
            }
        }
        tx.commit().unwrap();
        drop(conn);
        let queries: Vec<String> = (0..10)
            .map(|index| match index % 3 {
                0 => words.word(),
                1 => format!("{} {}", words.word(), words.word()),
                _ => format!("\"{} {}\"", words.word(), words.word()),
            })
            .collect();

        let mut startups = Vec::new();
        for _ in 0..10 {
            let started = Instant::now();
            let mut session = Session::start(dir.path());
            session.initialize();
            startups.push(started.elapsed());
            session.finish();
        }

        let mut session = Session::start(dir.path());
        session.initialize();
        session.call("search", json!({"query": "ba"}));
        let mut hits = Vec::new();
        let searches: Vec<Duration> = queries
            .iter()
            .map(|query| {
                let started = Instant::now();
                let found = session.call("search", json!({"query": query, "limit": 20}));
                let elapsed = started.elapsed();
                hits.push(found["meetings"].as_array().unwrap().len());
                elapsed
            })
            .collect();
        session.finish();

        println!(
            "murmur mcp: first initialize response p50 {:?} of 10 (process start included)",
            median(startups)
        );
        println!(
        "murmur mcp: search p50 {:?} of 10 on {MEETINGS} meetings x {TURNS} turns (meetings returned per query: {hits:?})",
        median(searches)
    );
    }
}
