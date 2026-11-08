use super::*;

// ---- The recording child ----

/// Time that goes on as fast as the audio is pushed.
#[derive(Clone)]
struct PushedClock {
    origin: Instant,
    unix_ms: i64,
    pushed_us: Arc<AtomicU64>,
}

impl PushedClock {
    fn pushed(&self) -> Duration {
        Duration::from_micros(self.pushed_us.load(Ordering::SeqCst))
    }
}

impl Clock for PushedClock {
    fn now(&self) -> Instant {
        self.origin + self.pushed()
    }

    fn unix_ms(&self) -> i64 {
        self.unix_ms + self.pushed().as_millis() as i64
    }
}

struct WriterTrack {
    writer: TrackWriter,
}

impl coordinator::Track for WriterTrack {
    fn failed(&self) -> bool {
        self.writer.failed()
    }

    fn interrupted(&self) -> bool {
        false
    }

    fn dropped_ms(&self) -> u64 {
        self.writer.dropped_ms()
    }

    fn finish(self: Box<Self>, duration: Duration) -> Result<bool, String> {
        self.writer.finish(duration).map(|_| false)
    }
}

/// Track writers with no device: the soak pushes their audio.
#[derive(Default, Clone)]
struct PushedAudio(Arc<Mutex<Vec<(Source, Feeder)>>>);

impl PushedAudio {
    pub(super) fn start(
        &self,
        source: Source,
        file: &'static str,
        dir: &Path,
        origin: Instant,
    ) -> Result<Box<dyn coordinator::Track>, String> {
        let writer = TrackWriter::create(&dir.join(file), origin)?;
        self.0.lock().unwrap().push((source, writer.feeder()));
        Ok(Box::new(WriterTrack { writer }))
    }
}

impl CaptureBackend for PushedAudio {
    fn system_audio(&self) -> super::super::capture::system::SystemAudioSupport {
        super::super::capture::system::SystemAudioSupport::Available
    }

    fn start_mic(
        &self,
        dir: &Path,
        origin: Instant,
    ) -> Result<Box<dyn coordinator::Track>, String> {
        self.start(Source::Mic, MIC_TRACK, dir, origin)
    }

    fn start_system(
        &self,
        dir: &Path,
        origin: Instant,
    ) -> Result<Box<dyn coordinator::Track>, String> {
        self.start(Source::System, SYSTEM_TRACK, dir, origin)
    }

    fn free_bytes(&self, _: &Path) -> Option<u64> {
        None
    }

    fn microphone_denied(&self) -> bool {
        false
    }

    fn system_audio_denied(&self) -> bool {
        false
    }

    fn keep_awake(&self) -> Option<Box<dyn Send>> {
        None
    }

    /// The dialogue's microphone holds the echo built-in speakers give.
    fn output_route(&self) -> OutputRoute {
        OutputRoute::Speakers
    }
}

/// Alerts go to stderr: the report counts what they are about.
struct Unseen;

impl Notifier for Unseen {
    fn runtime_changed(&self, _: &MeetingRuntime) {}
    fn alert(&self, alert: MeetingAlert) {
        eprintln!("The soak recording raised {alert:?}");
    }
    fn recorded(&self, _: i64) {}
}

/// The dialogue played on a loop, a little later each time round so the
/// speaker models never see the same copy twice.
struct Looped {
    audio: Vec<f32>,
    round: usize,
    at: usize,
}

impl Looped {
    pub(super) fn new(audio: Vec<f32>) -> Self {
        Self {
            audio,
            round: 0,
            at: 0,
        }
    }

    pub(super) fn take(&mut self, count: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let gap = (RATE as usize / 1000) * (200 + 150 * (self.round % 3));
            if self.at >= self.audio.len() + gap {
                self.round += 1;
                self.at = 0;
                continue;
            }
            out.push(self.audio.get(self.at).copied().unwrap_or(0.0));
            self.at += 1;
        }
        out
    }
}

pub(super) fn read_wav(path: &Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path)
        .unwrap_or_else(|error| panic!("Cannot read {}: {error}", path.display()));
    match reader.spec().sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / f32::from(i16::MAX))
            .collect(),
    }
}

pub(super) fn record(
    data: &Path,
    dialogue: &Path,
    seconds: u64,
    speed: u32,
    real_time_from_ms: Option<u64>,
) {
    let library = Library::open(data).unwrap();
    let clock = PushedClock {
        origin: Instant::now(),
        unix_ms: chrono::Utc::now().timestamp_millis(),
        pushed_us: Arc::default(),
    };
    let audio = PushedAudio::default();
    let mut coordinator = Coordinator::new(
        audio.clone(),
        Unseen,
        clock.clone(),
        Arc::new(library.store),
        library.root,
    );
    let id = coordinator.start(MeetingCaptureMode::Both).unwrap();
    let mut tracks: Vec<(Feeder, Looped)> = std::mem::take(&mut *audio.0.lock().unwrap())
        .into_iter()
        .map(|(source, feeder)| {
            let file = match source {
                Source::Mic => MIC_TRACK,
                Source::System => SYSTEM_TRACK,
            };
            (feeder, Looped::new(read_wav(&dialogue.join(file))))
        })
        .collect();
    assert_eq!(tracks.len(), 2, "both tracks record");

    let total_ms = seconds * 1000;
    let step = (RATE as u64 * STEP_MS / 1000) as usize;
    let started = Instant::now();
    let mut paced = Duration::ZERO;
    let mut pushed_ms = 0;
    while pushed_ms < total_ms {
        pushed_ms += STEP_MS;
        let captured = clock.origin + Duration::from_millis(pushed_ms);
        for (feeder, audio) in &mut tracks {
            feeder.push_at(audio.take(step), RATE, captured);
        }
        clock.pushed_us.store(pushed_ms * 1000, Ordering::SeqCst);
        if pushed_ms % 1000 == 0 && pushed_ms < total_ms {
            assert!(coordinator.tick(id), "the meeting stopped recording");
        }
        println!(
            "{LINE}{}",
            serde_json::to_string(&Recorded { id, pushed_ms }).unwrap()
        );
        let real_time = real_time_from_ms.is_some_and(|from| pushed_ms >= from);
        paced += Duration::from_millis(STEP_MS) / if real_time { 1 } else { speed };
        if let Some(ahead) = paced.checked_sub(started.elapsed()) {
            std::thread::sleep(ahead);
        }
    }
    coordinator.stop().unwrap();
}

// ---- The processing child ----

/// Keeps the log lines processing writes, and echoes them to stderr.
struct KeptLog(Mutex<Vec<String>>);

impl log::Log for KeptLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info && metadata.target().starts_with("murmur")
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let line = format!("{} {}", record.level(), record.args());
            eprintln!("{line}");
            self.0.lock().unwrap().push(line);
        }
    }

    fn flush(&self) {}
}

struct NoHost;

impl Host for NoHost {
    fn notify(&self, _: LocalModelStatus) {}
    fn remember_wanted(&self, _: bool) {}
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        None
    }
    fn prepare(&self, _: PathBuf, _: ProcessingOperation) -> PrepareFuture {
        Box::pin(async { Prepared::Failed })
    }
}

/// What the app does without the models: one speaker per track.
pub(super) struct NoModels;

impl Analyzer for NoModels {
    fn analyze(&self, _: AnalysisRequest, _: ProcessingOperation) -> AnalyzeFuture {
        Box::pin(async { AnalyzerOutcome::Unavailable(SpeakerAnalysis::NoModels) })
    }
}

pub(super) struct CannedSummary;

impl Summarizer for CannedSummary {
    fn complete(&self, _: String, _: String, _: ProcessingOperation) -> CompleteFuture {
        Box::pin(async { Ok("# Soak meeting\nNotes.".to_string()) })
    }
}

pub(super) struct NoEvents;

impl ProcessEvents for NoEvents {
    fn progress(&self, _: MeetingProgress) {}
    fn updated(&self, _: i64) {}
    fn finished(&self, _: super::super::announce::Finished) {}
}

pub(super) fn process(data: &Path, codex: Option<(String, PathBuf)>, models: Option<PathBuf>) {
    let kept: &'static KeptLog = Box::leak(Box::new(KeptLog(Mutex::default())));
    log::set_logger(kept).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    let started = Instant::now();
    let library = Library::open(data).unwrap();
    let store = Arc::new(library.store);
    let waiting = store.waiting_for_processing().unwrap();
    let analyzer: Arc<dyn Analyzer> = match models {
        Some(dir) => {
            let models = LocalModels::new(
                dir,
                manifest::this_mac(),
                helper::support(),
                manifest::HUGGING_FACE.to_string(),
                Arc::new(NoHost),
            );
            assert!(
                models.is_ready(),
                "the models are not ready: {:?}",
                models.status()
            );
            platform_analyzer(models)
        }
        None => Arc::new(NoModels),
    };
    let (url, auth_path) = match codex {
        Some((url, auth)) => (url, Some(auth)),
        None => (crate::codex_transcribe::TRANSCRIBE_URL.to_string(), None),
    };
    for id in &waiting {
        let processor = Processor {
            store: Arc::clone(&store),
            root: library.root.clone(),
            transcriber: Arc::new(CodexTranscriber {
                url: url.clone(),
                auth_path: auth_path.clone(),
            }),
            local: None,
            analyzer: Arc::clone(&analyzer),
            summarizer: Arc::new(CannedSummary),
            policy: RetryPolicy::default(),
            language: Some("fr".to_string()),
            vocabulary: Vec::new(),
            events: Arc::new(NoEvents),
            compress_audio: true,
        };
        let operation = ProcessingOperation::new(OperationId(*id as u64));
        tauri::async_runtime::block_on(processor.process(*id, operation)).unwrap();
    }
    let log = kept
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|line| {
            ["timings:", "echo(es)", "Meeting analysis:", "processed:"]
                .iter()
                .any(|kept| line.contains(kept))
        })
        .cloned()
        .collect();
    let processed = Processed {
        recovered: library.recovered,
        waiting,
        seconds: started.elapsed().as_secs_f64(),
        footprint: peak_footprint(),
        helper_rss: children_peak_rss(),
        log,
    };
    println!("{LINE}{}", serde_json::to_string(&processed).unwrap());
}
