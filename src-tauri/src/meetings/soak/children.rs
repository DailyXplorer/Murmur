use super::*;

/// The peak resident memory of the largest child waited for, in bytes.
#[cfg(target_os = "macos")]
pub(super) fn children_peak_rss() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` is a valid rusage to write into.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()) },
        0
    );
    // SAFETY: getrusage succeeded, and the struct was zeroed before.
    unsafe { usage.assume_init() }.ru_maxrss as u64
}

/// Windows analyses in the app: there is no helper process to measure.
#[cfg(target_os = "windows")]
pub(super) fn children_peak_rss() -> u64 {
    0
}

/// This process's peak footprint, in bytes. Unlike its resident size, it
/// leaves out the pages of the test binary itself, which are large in a
/// debug build.
#[cfg(target_os = "macos")]
pub(super) fn peak_footprint() -> u64 {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    // SAFETY: `info` is a valid rusage_info_v4 to write into, as the flavor says.
    let read = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as libc::c_int,
            libc::RUSAGE_INFO_V4,
            info.as_mut_ptr().cast(),
        )
    };
    assert_eq!(read, 0);
    // SAFETY: proc_pid_rusage succeeded, and the struct was zeroed before.
    unsafe { info.assume_init() }.ri_lifetime_max_phys_footprint
}

/// The most memory this process had committed, in bytes: what it asked
/// Windows for, which is what the page file would have to hold.
#[cfg(target_os = "windows")]
pub(super) fn peak_footprint() -> u64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size,
        ..Default::default()
    };
    // SAFETY: `counters` is a valid PROCESS_MEMORY_COUNTERS of `size` bytes,
    // and the pseudo handle of the current process needs no closing.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) }
        .expect("the process reads its own memory counters");
    counters.PeakPagefileUsage as u64
}

// ---- The parent ----

/// A child run of the test binary.
pub(super) struct Running {
    child: std::process::Child,
    pub(super) lines: Receiver<String>,
}

pub(super) fn spawn(exe: &Path, child: &Child) -> Running {
    let mut command = Command::new(exe);
    command
        .args([
            "meetings::soak::soak_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, serde_json::to_string(child).unwrap())
        .stdout(Stdio::piped());
    if matches!(
        child,
        Child::Process {
            release_freed: true,
            ..
        }
    ) {
        command.envs(RELEASE_FREED);
    }
    let mut process = command.spawn().unwrap();
    let stdout = process.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            // The first follows libtest's "test soak_child ... " line.
            if let Some((_, json)) = line.split_once(LINE) {
                if sender.send(json.to_string()).is_err() {
                    return;
                }
            }
        }
    });
    Running {
        child: process,
        lines,
    }
}

impl Running {
    /// Waits for the child to end well, and returns the last line it printed.
    pub(super) fn finish<T: serde::de::DeserializeOwned>(mut self) -> T {
        let status = self.child.wait().unwrap();
        assert!(status.success(), "the soak child failed: {status}");
        let last = self
            .lines
            .iter()
            .last()
            .expect("the soak child printed nothing");
        serde_json::from_str(&last).unwrap()
    }

    fn kill(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

pub(super) struct Recording {
    pub(super) id: i64,
    /// Audio pushed to each track before it ended.
    pub(super) pushed_ms: u64,
}

/// Records `seconds` of the dialogue in `data`, or until the microphone
/// track holds `kill_at_ms` on disk, where the recording is killed.
pub(super) fn record_in(
    exe: &Path,
    data: &Path,
    dialogue: &Path,
    seconds: u64,
    kill_at_ms: Option<u64>,
) -> Recording {
    let mut running = spawn(
        exe,
        &Child::Record {
            data: data.to_path_buf(),
            dialogue: dialogue.to_path_buf(),
            seconds,
            speed: 100,
            real_time_from_ms: kill_at_ms.map(|at| at.saturating_sub(REAL_TIME_BEFORE_KILL_MS)),
        },
    );
    let Some(kill_at_ms) = kill_at_ms else {
        let last: Recorded = running.finish();
        assert_eq!(last.pushed_ms, seconds * 1000);
        return Recording {
            id: last.id,
            pushed_ms: last.pushed_ms,
        };
    };
    let mut last: Option<Recorded> = None;
    loop {
        match running.lines.recv_timeout(Duration::from_millis(10)) {
            Ok(line) => last = Some(serde_json::from_str(&line).unwrap()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("the recording ended before its kill"),
        }
        let Some(Recorded { id, .. }) = last else {
            continue;
        };
        let mic = paths::meeting_dir(&data.join("meetings"), id).join(MIC_TRACK);
        let on_disk_ms =
            std::fs::metadata(&mic).map_or(0, |file| file.len().saturating_sub(44) / 32);
        if on_disk_ms >= kill_at_ms {
            break;
        }
    }
    running.kill();
    let last = running
        .lines
        .iter()
        .map(|line| serde_json::from_str::<Recorded>(&line).unwrap())
        .last()
        .or(last)
        .unwrap();
    Recording {
        id: last.id,
        pushed_ms: last.pushed_ms,
    }
}

/// Processes the library in `data` until `done_share` of meeting `id`'s
/// pieces with speech are transcribed, and kills it there. Returns the
/// pieces it saved as transcribed.
pub(super) fn process_until_killed(
    exe: &Path,
    child: &Child,
    store: &MeetingsStore,
    id: i64,
    done_share: f64,
) -> Vec<u64> {
    let mut running = spawn(exe, child);
    let done = |store: &MeetingsStore| -> Option<Vec<u64>> {
        let (speech, done) = transcribed(store, id)?;
        (speech > 0 && done.len() as f64 >= (speech as f64 * done_share).max(1.0)).then_some(done)
    };
    loop {
        assert!(
            running.child.try_wait().unwrap().is_none(),
            "processing ended before its kill"
        );
        if done(store).is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    running.kill();
    done(store).expect("the transcribed pieces stay saved")
}

/// How many pieces of meeting `id` hold speech, and the audio of those saved
/// as transcribed.
pub(super) fn transcribed(store: &MeetingsStore, id: i64) -> Option<(usize, Vec<u64>)> {
    let chunks = store.chunks(id).ok()?;
    let speech = chunks
        .iter()
        .filter(|chunk| chunk.status != ChunkStatus::Silent)
        .count();
    let done = chunks
        .iter()
        .filter(|chunk| chunk.status == ChunkStatus::Done)
        .filter_map(|chunk| piece_of(chunk.text.as_deref()?))
        .collect();
    Some((speech, done))
}

/// How many times a relaunch sent Codex a piece already transcribed before
/// it: `sent` sends of the audio in `done_before`, beyond the pieces of that
/// audio still to do. Counted per audio, as two pieces can hold the same.
pub(super) fn sent_again(done_before: &[u64], sent: &[u64], done_after: &[u64]) -> usize {
    let count = |pieces: &[u64], piece: u64| pieces.iter().filter(|&&other| other == piece).count();
    done_before
        .iter()
        .copied()
        .collect::<HashSet<u64>>()
        .into_iter()
        .map(|piece| {
            (count(done_before, piece) + count(sent, piece))
                .saturating_sub(count(done_after, piece))
        })
        .sum()
}

/// What one meeting ended as.
pub(super) struct Outcome {
    pub(super) status: MeetingStatus,
    pub(super) duration_ms: i64,
    /// Bubbles per speaker key.
    pub(super) turns: BTreeMap<String, usize>,
    pub(super) analysis: Option<SpeakerAnalysis>,
}

pub(super) fn outcome(store: &MeetingsStore, id: i64) -> Outcome {
    let meeting = store.get(id).unwrap().unwrap();
    let mut turns = BTreeMap::new();
    for turn in &meeting.turns {
        *turns.entry(turn.speaker_key.clone()).or_insert(0) += 1;
    }
    Outcome {
        status: meeting.status,
        duration_ms: meeting.duration_ms,
        turns,
        analysis: meeting.speaker_analysis,
    }
}

impl Outcome {
    /// The user and at least one remote voice have bubbles.
    pub(super) fn both_speakers(&self) -> bool {
        self.turns.contains_key("me")
            && self
                .turns
                .keys()
                .any(|key| key != "me" && key != PLACEHOLDER)
    }
}

/// The samples each track of a recovered meeting holds, in milliseconds.
pub(super) fn track_ms(data: &Path, id: i64) -> [u64; 2] {
    let dir = paths::meeting_dir(&data.join("meetings"), id);
    [MIC_TRACK, SYSTEM_TRACK].map(|file| {
        let reader = hound::WavReader::open(dir.join(file)).unwrap();
        u64::from(reader.duration()) * 1000 / u64::from(RATE)
    })
}

/// The two tracks of a dialogue, written as `mic.wav` and `system.wav`:
/// voice A on the microphone, voice B on system audio, and B's echo on the
/// microphone, as built-in speakers give it. Returns where each turn starts,
/// in samples.
pub(super) fn write_dialogue(dir: &Path, turns: &[(bool, Vec<f32>)]) -> Vec<usize> {
    let pause = |index: usize| RATE as usize / 1000 * (500 + 250 * (index % 3));
    let length: usize = turns
        .iter()
        .enumerate()
        .map(|(index, (_, clip))| clip.len() + pause(index))
        .sum::<usize>()
        + ECHO_DELAY_SAMPLES;
    let (mut mic, mut system) = (vec![0f32; length], vec![0f32; length]);
    let mut at = 0;
    let mut starts = Vec::with_capacity(turns.len());
    for (index, (remote, clip)) in turns.iter().enumerate() {
        starts.push(at);
        for (offset, sample) in clip.iter().enumerate() {
            if *remote {
                system[at + offset] += sample;
                mic[at + offset + ECHO_DELAY_SAMPLES] += sample * ECHO_GAIN;
            } else {
                mic[at + offset] += sample;
            }
        }
        at += clip.len() + pause(index);
    }
    std::fs::create_dir_all(dir).unwrap();
    for (file, track) in [(MIC_TRACK, mic), (SYSTEM_TRACK, system)] {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(dir.join(file), spec).unwrap();
        for sample in track {
            let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
    }
    starts
}

/// Alternating tones that read as speech, for the quick variant.
pub(super) fn tone_dialogue(dir: &Path, seconds: usize) {
    let turns: Vec<(bool, Vec<f32>)> = (0..seconds / 5)
        .map(|index| {
            let pitch = if index % 2 == 0 { 0.07 } else { 0.11 };
            let clip = (0..4 * RATE as usize)
                .map(|sample| (sample as f32 * pitch).sin() * 0.4)
                .collect();
            (index % 2 == 1, clip)
        })
        .collect();
    write_dialogue(dir, &turns);
}
