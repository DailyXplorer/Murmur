use super::*;

// ---- The three-hour soak ----

/// The voices of the dialogue: A is the user, B the remote side.
#[cfg(target_os = "macos")]
pub(super) const VOICE_A: &str = "Thomas";

#[cfg(target_os = "macos")]
pub(super) const VOICE_B: &str = "Amélie";

/// Says the dialogue into `dir`, unless it is there already. `say` writes
/// to files only: nothing plays.
#[cfg(target_os = "macos")]
pub(super) fn say_dialogue(dir: &Path) {
    if dir.join(MIC_TRACK).is_file() && dir.join(SYSTEM_TRACK).is_file() {
        return;
    }
    let turns: Vec<(bool, Vec<f32>)> = said_lines()
        .into_iter()
        .map(|((remote, _), clip)| (remote, clip))
        .collect();
    write_dialogue(dir, &turns);
}

/// Each line of the dialogue, said three times at different rates, with
/// its audio.
#[cfg(target_os = "macos")]
pub(super) fn said_lines() -> Vec<((bool, &'static str), Vec<f32>)> {
    let clips = tempfile::tempdir().unwrap();
    let mut said = Vec::new();
    for (round, rate) in [175, 195, 215].into_iter().enumerate() {
        for (index, &(remote, line)) in LINES.iter().enumerate() {
            let clip = clips.path().join(format!("{round}-{index}.wav"));
            let status = Command::new("say")
                .args(["-v", if remote { VOICE_B } else { VOICE_A }])
                .args(["-r", &rate.to_string(), "-o"])
                .arg(&clip)
                .args(["--file-format=WAVE", "--data-format=LEF32@16000", line])
                .status()
                .unwrap();
            assert!(status.success(), "say failed: {status}");
            said.push(((remote, line), read_wav(&clip)));
        }
    }
    said
}

/// The analysis stage's time, read from the timings line.
#[cfg(target_os = "macos")]
pub(super) fn stage_seconds(processed: &Processed, stage: &str) -> Option<f64> {
    let line = processed
        .log
        .iter()
        .find(|line| line.contains("timings:"))?;
    let (_, after) = line.split_once(&format!("{stage} "))?;
    after.split_once(" s")?.0.parse().ok()
}

#[cfg(target_os = "macos")]
pub(super) fn echo_removals(processed: &Processed) -> usize {
    processed
        .log
        .iter()
        .filter_map(|line| {
            let (_, after) = line.split_once("Removed ")?;
            after.split_once(" echo")?.0.parse::<usize>().ok()
        })
        .sum()
}

#[cfg(target_os = "macos")]
pub(super) fn sh(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(target_os = "macos")]
pub(super) fn gib(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / GIB as f64)
}

#[cfg(target_os = "macos")]
pub(super) fn minutes(ms: u64) -> String {
    format!(
        "{}:{:02}:{:02}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60
    )
}

/// One check of the report, and whether it passed.
#[cfg(target_os = "macos")]
pub(super) struct Check {
    pub(super) what: String,
    pub(super) passed: bool,
}

#[cfg(target_os = "macos")]
pub(super) fn check(what: impl Into<String>, passed: bool) -> Check {
    Check {
        what: what.into(),
        passed,
    }
}

// ---- Echoes without the on-device models ----

/// Codex as a script: each piece is answered with the lines said in it, the
/// echoes the microphone heard included, found by where its audio sits in
/// the track.
#[cfg(target_os = "macos")]
pub(super) struct ScriptedCodex {
    pub(super) tracks: Vec<ScriptedTrack>,
}

#[cfg(target_os = "macos")]
pub(super) struct ScriptedTrack {
    pub(super) samples: Vec<f32>,
    /// Where each line said on the track starts, in samples.
    pub(super) lines: Vec<(usize, &'static str)>,
}

#[cfg(target_os = "macos")]
impl ScriptedCodex {
    pub(super) fn answer(&self, piece: &[f32]) -> String {
        let Some(first) = piece.iter().position(|sample| *sample != 0.0) else {
            return String::new();
        };
        let probe = &piece[first..(first + 256).min(piece.len())];
        for ScriptedTrack { samples, lines } in &self.tracks {
            if let Some(at) = samples
                .windows(probe.len())
                .position(|window| window == probe)
            {
                let start = at - first;
                return lines
                    .iter()
                    .filter(|(line_at, _)| (start..start + piece.len()).contains(line_at))
                    .map(|(_, line)| *line)
                    .collect::<Vec<_>>()
                    .join(" ");
            }
        }
        panic!("a piece is in neither track");
    }
}

#[cfg(target_os = "macos")]
impl Transcriber for ScriptedCodex {
    fn transcribe(
        &self,
        samples: Vec<f32>,
        _: Option<String>,
        _: ProcessingOperation,
    ) -> TranscribeFuture {
        let text = self.answer(&samples);
        Box::pin(async move { Ok(text) })
    }
}

/// How often the words of `line` come in a row in `keys`, a run of word
/// keys with a space on either side.
#[cfg(target_os = "macos")]
pub(super) fn said_in(keys: &str, line: &str) -> usize {
    let line: Vec<String> = tokenize(line)
        .into_iter()
        .map(|token| token.key)
        .filter(|key| !key.is_empty())
        .collect();
    keys.matches(&format!(" {} ", line.join(" "))).count()
}

/// The length of the dialogue's microphone track.
#[cfg(target_os = "macos")]
pub(super) fn track_ms_in(dialogue: &Path) -> u64 {
    let reader = hound::WavReader::open(dialogue.join(MIC_TRACK)).unwrap();
    u64::from(reader.duration()) * 1000 / u64::from(RATE)
}

#[cfg(target_os = "macos")]
pub(super) fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is set"))
}
