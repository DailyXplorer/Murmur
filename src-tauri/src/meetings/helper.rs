//! Runs `murmur-meetings-helper`, the separate process that does the
//! on-device analysis (FluidAudio needs macOS 15, and a crash in Core ML
//! must not take the app down). Its stdout is read line by line as
//! `protocol` messages; its stderr goes to the debug log only.

use serde::Serialize;
use specta::Type;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::protocol::{parse_line, Message};
use crate::ProcessingOperation;

pub const HELPER_NAME: &str = "murmur-meetings-helper";

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisSupport {
    Available,
    /// The analysis needs macOS 15 or later: on macOS 14 FluidAudio's
    /// speaker diarizer crashes on every run.
    OsTooOld,
    /// Intel Macs: one speaker per track.
    #[cfg_attr(target_arch = "aarch64", allow(dead_code))]
    Unsupported,
}

#[cfg(target_arch = "aarch64")]
extern "C" {
    fn murmur_meeting_analysis_support() -> i32;
}

pub fn support() -> AnalysisSupport {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: takes no arguments and only reads the OS version.
        match unsafe { murmur_meeting_analysis_support() } {
            0 => AnalysisSupport::Available,
            _ => AnalysisSupport::OsTooOld,
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        AnalysisSupport::Unsupported
    }
}

/// Tauri puts the helper next to the app's executable, in development too.
pub fn helper_path() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?.parent()?.join(HELPER_NAME);
    path.is_file().then_some(path)
}

/// How a helper run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// It printed `done`: everything it could do was reported.
    Done,
    /// It stopped without `done`: a crash, or killed by the system.
    Crashed,
    Cancelled,
    TimedOut,
}

/// Runs `program` with `args`, handing each message to `on_message`. Lines
/// that are not messages are logged and skipped.
pub async fn run(
    program: &Path,
    args: &[OsString],
    operation: &ProcessingOperation,
    timeout: Duration,
    mut on_message: impl FnMut(Message),
) -> std::io::Result<Ended> {
    // The helper exits when its stdin closes. Murmur never writes to it, and
    // the system closes it when Murmur quits, even before a kill could run.
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let _stdin = child.stdin.take();
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log::debug!("Meetings helper: {line}");
            }
        });
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("the helper has no stdout"))?;
    let mut lines = BufReader::new(stdout).lines();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut done = false;
    loop {
        tokio::select! {
            line = lines.next_line() => match line? {
                Some(line) if line.trim().is_empty() => {}
                Some(line) => match parse_line(&line) {
                    Ok(Message::Done) => done = true,
                    Ok(message) => on_message(message),
                    Err(error) => log::warn!("Meetings helper printed an unreadable line: {error}"),
                },
                None => break,
            },
            () = operation.cancelled() => {
                let _ = child.kill().await;
                return Ok(Ended::Cancelled);
            }
            () = &mut deadline => {
                let _ = child.kill().await;
                return Ok(Ended::TimedOut);
            }
        }
    }
    let status = child.wait().await?;
    Ok(if done && status.success() {
        Ended::Done
    } else {
        log::warn!("Meetings helper stopped without finishing: {status}");
        Ended::Crashed
    })
}

/// The helper is stood in for by shell scripts, so these run on macOS.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::meetings::merge::Track;
    use crate::OperationId;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in helper: a shell script with the given body.
    fn script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("helper.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn run_script(
        body: &str,
        timeout: Duration,
        operation: &ProcessingOperation,
    ) -> (Ended, Vec<Message>) {
        let dir = tempfile::tempdir().unwrap();
        let program = script(dir.path(), body);
        let mut messages = Vec::new();
        let ended = tauri::async_runtime::block_on(run(&program, &[], operation, timeout, |m| {
            messages.push(m)
        }))
        .unwrap();
        (ended, messages)
    }

    fn operation() -> ProcessingOperation {
        ProcessingOperation::new(OperationId(1))
    }

    #[test]
    fn messages_arrive_in_order_and_done_ends_the_run() {
        let (ended, messages) = run_script(
            r#"echo '{"type":"progress","phase":"asr","track":"mic","fraction":0}'
echo 'a log line that is not JSON'
echo ''
echo '{"type":"words","track":"mic","words":[]}'
echo '{"type":"done"}'"#,
            Duration::from_secs(10),
            &operation(),
        );
        assert_eq!(ended, Ended::Done);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[1],
            Message::Words {
                track: Track::Mic,
                words: vec![]
            }
        );
    }

    #[test]
    fn the_helper_stdin_stays_open_while_murmur_waits_for_it() {
        // The helper exits when its stdin closes, which is how it learns
        // that Murmur quit; while the run lasts, stdin must stay open.
        let (ended, messages) = run_script(
            r#"cat > /dev/null
echo '{"type":"done"}'"#,
            Duration::from_millis(500),
            &operation(),
        );
        assert_eq!(ended, Ended::TimedOut);
        assert!(messages.is_empty());
    }

    #[test]
    fn a_helper_that_dies_mid_way_keeps_what_it_said() {
        let (ended, messages) = run_script(
            r#"echo '{"type":"words","track":"system","words":[]}'
kill -9 $$"#,
            Duration::from_secs(10),
            &operation(),
        );
        assert_eq!(ended, Ended::Crashed);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn a_failing_exit_after_done_is_a_crash() {
        let (ended, _) = run_script(
            r#"echo '{"type":"done"}'
exit 3"#,
            Duration::from_secs(10),
            &operation(),
        );
        assert_eq!(ended, Ended::Crashed);
    }

    #[test]
    fn a_hung_helper_is_stopped_at_the_deadline() {
        let (ended, _) = run_script("exec sleep 30", Duration::from_millis(300), &operation());
        assert_eq!(ended, Ended::TimedOut);
    }

    #[test]
    fn cancelling_stops_the_helper() {
        let cancelled = operation();
        cancelled.cancel();
        let (ended, _) = run_script("exec sleep 30", Duration::from_secs(10), &cancelled);
        assert_eq!(ended, Ended::Cancelled);
    }
}
