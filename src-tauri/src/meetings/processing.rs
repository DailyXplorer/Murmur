use super::*;

impl RateLimitHold {
    const FIRST: Duration = Duration::from_secs(30 * 60);
    const CAP: Duration = Duration::from_secs(4 * 3600);

    /// The wait after one more refusal.
    pub(super) fn rate_limited(&mut self) -> Duration {
        let wait = Self::FIRST
            .saturating_mul(1 << self.in_a_row.min(8))
            .min(Self::CAP);
        self.in_a_row += 1;
        wait
    }

    /// Codex answered: the cap has freed up.
    pub(super) fn reset(&mut self) {
        self.in_a_row = 0;
    }
}

impl ProcessEvents for AppEvents {
    fn progress(&self, progress: MeetingProgress) {
        if let Err(error) = progress.emit(&self.0) {
            error!("Failed to emit meeting progress: {error}");
        }
    }

    fn updated(&self, id: i64) {
        if let Err(error) = (MeetingsUpdated { id }).emit(&self.0) {
            error!("Failed to emit a meeting update: {error}");
        }
        crate::overlay::meeting_updated(&self.0, id);
    }

    fn finished(&self, finished: announce::Finished) {
        crate::overlay::announce_meeting(&self.0, finished);
    }
}

/// Ends a run, retaining unfinished work separately from the verdict that
/// holds the queue. A transcript and its summary may end for different reasons.
pub(super) fn processing_finished(
    worker: &ProcessingQueue,
    store: &MeetingsStore,
    id: i64,
    codex: CodexVerdict,
    local_tried: bool,
) {
    worker.end();
    // While held, the meetings still queued wait as they are,
    // unfailed.
    match (codex, worker.codex_said(codex)) {
        (CodexVerdict::Unreachable, Some(wait)) => {
            info!("Meeting {id}: Codex is out of reach; checking the connection again in {wait:?}");
            // It goes on with the others once the connection is back.
            worker.requeue(id, local_tried || codex_only(store, id));
        }
        (_, Some(wait)) => {
            info!("Meeting {id}: Codex is rate limiting; holding the queue for {wait:?}");
            // A chunk's rate limit can outweigh a later unreachable summary.
            // Keep that Pending summary behind the cap, without automatically
            // retrying a Failed summary. Only Codex can finish it.
            if codex_only(store, id) {
                worker.requeue(id, true);
            }
        }
        (_, None) => {}
    }
}

/// Runs the queue for the life of the app.
pub(super) fn spawn_processing(
    app: &AppHandle,
    store: Arc<MeetingsStore>,
    root: PathBuf,
    models: Arc<Models>,
) -> Arc<ProcessingQueue> {
    let mut queue = ProcessingQueue::new(!get_settings(app).meetings_enabled);
    let ready = Arc::clone(&models);
    queue.local_ready = Box::new(move || ready.is_ready());
    let queue = Arc::new(queue);
    let mut model_state = models.subscribe();
    let waking = Arc::clone(&queue);
    tauri::async_runtime::spawn(async move {
        while model_state.changed().await.is_ok() {
            if *model_state.borrow_and_update() == LocalModelState::Ready {
                waking.local_model_ready();
            }
        }
    });
    let worker = Arc::clone(&queue);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut operations = 0;
        loop {
            let id = worker.next().await;
            operations += 1;
            let operation = ProcessingOperation::new(OperationId(operations));
            if !worker.begin(id, &operation) {
                continue;
            }
            // Its view stops saying it waits for a hold or the connection.
            AppEvents(app.clone()).updated(id);
            let settings = get_settings(&app);
            let (language, vocabulary) = (settings.selected_language, settings.custom_words);
            let local_tried = models.is_ready();
            let processor = Processor {
                store: Arc::clone(&store),
                root: root.clone(),
                transcriber: Arc::new(CodexTranscriber {
                    url: crate::codex_transcribe::TRANSCRIBE_URL.to_string(),
                    auth_path: None,
                }),
                // Its own engine, released with the run.
                local: local_tried
                    .then(|| crate::engine::local::engine(&app, Box::new(|| false)))
                    .flatten()
                    .map(|engine| {
                        Arc::new(LocalTranscriber(Arc::new(engine))) as Arc<dyn Transcriber>
                    }),
                analyzer: analysis::platform_analyzer(Arc::clone(&models)),
                summarizer: Arc::new(CodexSummarizer {
                    url: crate::cleanup::responses::RESPONSES_URL.to_string(),
                    auth_path: None,
                }),
                policy: RetryPolicy::default(),
                language: Some(language),
                vocabulary,
                events: Arc::new(AppEvents(app.clone())),
                compress_audio: true,
            };
            info!("Meeting {id}: processing");
            let codex = match processor.process(id, operation.clone()).await {
                Ok(codex) => codex,
                Err(error) => {
                    error!("Meeting {id}: processing failed: {error}");
                    if !operation.is_cancelled() {
                        if let Err(error) =
                            processor.finish_processing(id, Some(ProcessingError::Other))
                        {
                            error!("Meeting {id}: cannot record the failure: {error}");
                        }
                    }
                    CodexVerdict::Unknown
                }
            };
            processing_finished(&worker, &store, id, codex, local_tried);
            // A hold that starts or lifts changes what their views say.
            if codex != CodexVerdict::Unknown {
                let events = AppEvents(app.clone());
                worker
                    .queued()
                    .into_iter()
                    .for_each(|id| events.updated(id));
            }
        }
    });
    queue
}

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

impl Notifier for AppNotifier {
    fn runtime_changed(&self, runtime: &MeetingRuntime) {
        *self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = runtime.clone();
        match runtime {
            MeetingRuntime::Idle => crate::overlay::hide_meeting_overlay(&self.app),
            MeetingRuntime::Recording {
                started_at_ms,
                stops_at_ms,
                ..
            } => crate::overlay::show_meeting_overlay(&self.app, *started_at_ms, *stops_at_ms),
        }
        crate::utils::update_tray_menu(&self.app, None);
        if let Err(error) = MeetingStateChanged(runtime.clone()).emit(&self.app) {
            error!("Failed to emit the meeting state: {error}");
        }
    }

    fn alert(&self, alert: MeetingAlert) {
        user_alert::show(&self.app, Alert::Meeting(alert));
    }

    fn recorded(&self, id: i64) {
        self.queue.enqueue(id);
    }
}
