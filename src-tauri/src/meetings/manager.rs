use super::*;

impl Library {
    /// Opens the library in `app_data`, sets aside what no meeting points
    /// to, and marks meetings a crash cut off as interrupted.
    pub(super) fn open(app_data: &Path) -> anyhow::Result<Self> {
        #[cfg(not(target_os = "windows"))]
        let root = app_data.join("meetings");
        #[cfg(target_os = "windows")]
        let root = crate::data_dirs_windows::meetings_dir(app_data);
        std::fs::create_dir_all(&root)?;
        // No import runs yet: what an earlier run left of one is abandoned.
        match import_audio::sweep(&root) {
            0 => {}
            count => info!("Removed {count} abandoned audio import(s)"),
        }
        let now = chrono::Utc::now();
        let database = app_data.join("meetings.db");
        let had_database = database.exists();
        let (store, startup) = MeetingsStore::open(database, now);
        info!("Meetings database: {startup:?}");
        let mut reset = false;
        match &startup {
            DatabaseStartup::Reset { moved_to } => {
                reset = true;
                if let Err(error) = paths::set_aside_recordings(&root, moved_to) {
                    error!("Failed to set aside the recordings of the reset library: {error}");
                }
            }
            // Only a library that opened says which folders no row points to.
            DatabaseStartup::Ready => {
                match paths::remove_stale(&root) {
                    0 => {}
                    count => info!("Removed {count} stale meeting folder(s)"),
                }
                let aside =
                    app_data.join(format!("meetings.orphaned-{}", now.format("%Y%m%d-%H%M%S")));
                // Beside the folder it comes from: a rename across volumes
                // would fail.
                #[cfg(target_os = "windows")]
                let aside = root.with_file_name(aside.file_name().unwrap_or_default());
                let moved = store
                    .ids()
                    .and_then(|ids| Ok(paths::set_aside_orphans(&root, &ids, &aside)?));
                match moved {
                    Ok(0) => {}
                    Ok(count) => {
                        warn!(
                            "Moved {count} meeting folder(s) no meeting points to into {}",
                            aside.display()
                        );
                        reset = !had_database;
                    }
                    Err(error) => error!("Failed to set aside orphan meeting folders: {error}"),
                }
            }
            DatabaseStartup::Unavailable => {}
        }
        match paths::purge_set_asides(app_data, now) {
            0 => {}
            count => {
                info!("Deleted {count} meeting folder(s) or file(s) set aside over 30 days ago")
            }
        }
        let recovered = store
            .recover_interrupted(&root)
            .inspect_err(|error| error!("Failed to recover interrupted meetings: {error}"))
            .unwrap_or_default();
        if !recovered.is_empty() {
            info!("Recovered interrupted meeting(s) {recovered:?}");
        }
        Ok(Self {
            store,
            root,
            recovered,
            reset,
        })
    }
}

impl Drop for ImportSlot<'_> {
    fn drop(&mut self) {
        *self.0.import_slot() = None;
        self.0.announce_import(None);
    }
}

impl MeetingsManager {
    /// Opens the library and marks meetings a crash cut off as interrupted.
    pub fn new(app: &AppHandle) -> anyhow::Result<Self> {
        let Library {
            store,
            root,
            recovered,
            reset,
        } = Library::open(&app.path().app_data_dir()?)?;
        if reset {
            user_alert::show(app, Alert::MeetingsReset);
        }
        let store = Arc::new(store);
        let models = app
            .try_state::<Arc<Models>>()
            .map(|models| Arc::clone(&models))
            .ok_or_else(|| anyhow::anyhow!("The local models are unavailable"))?;
        let queue = spawn_processing(app, Arc::clone(&store), root.clone(), Arc::clone(&models));
        let runtime = Arc::new(Mutex::new(MeetingRuntime::Idle));
        let emitter = app.clone();
        app.manage(Arc::new(live::Live::new(Box::new(move |update| {
            if let Err(error) = update.emit(&emitter) {
                error!("Failed to emit the live transcript: {error}");
            }
        }))));
        let notifier = AppNotifier {
            app: app.clone(),
            runtime: Arc::clone(&runtime),
            queue: Arc::clone(&queue),
        };
        Ok(Self {
            app: app.clone(),
            coordinator: Mutex::new(Coordinator::new(
                DeviceCapture(app.clone()),
                notifier,
                SystemClock,
                Arc::clone(&store),
                root.clone(),
            )),
            runtime,
            store,
            queue,
            root,
            models,
            recovered,
            import: Mutex::new(None),
        })
    }

    fn coordinator(&self) -> std::sync::MutexGuard<'_, AppCoordinator> {
        self.coordinator
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn runtime(&self) -> MeetingRuntime {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn import_slot(&self) -> std::sync::MutexGuard<'_, Option<ActiveImport>> {
        self.import
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn announce_import(&self, import: Option<MeetingImport>) {
        if let Err(error) = MeetingImportChanged(import).emit(&self.app) {
            error!("Failed to emit the audio import: {error}");
        }
    }

    /// The audio import under way, for a page opened meanwhile.
    pub fn current_import(&self) -> Option<MeetingImport> {
        self.import_slot()
            .as_ref()
            .map(|active| active.shown.clone())
    }

    /// Stops the audio import under way, if any; it leaves nothing behind.
    pub fn cancel_import(&self) {
        if let Some(active) = self.import_slot().as_ref() {
            active.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Makes a meeting of the audio file `file` and queues it for
    /// processing. Blocks until the file is decoded. `Ok(None)`: the user
    /// cancelled.
    pub fn import_audio(&self, file: &Path) -> Result<Option<i64>, ImportError> {
        if !get_settings(&self.app).meetings_enabled {
            return Err(ImportError::Disabled);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let file_name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shown = MeetingImport {
            file_name,
            fraction: 0.0,
        };
        {
            let mut slot = self.import_slot();
            if slot.is_some() {
                return Err(ImportError::Busy);
            }
            *slot = Some(ActiveImport {
                cancel: Arc::clone(&cancel),
                shown: shown.clone(),
            });
        }
        let _slot = ImportSlot(self);
        self.announce_import(Some(shown.clone()));
        let source = import_audio::Source {
            file,
            free_bytes: capture::free_bytes(&self.root),
            now_ms: chrono::Utc::now().timestamp_millis(),
        };
        let result = import_audio::import(
            &self.store,
            &self.root,
            &source,
            &cancel,
            import_audio::throttled(|fraction| {
                let shown = MeetingImport {
                    fraction,
                    ..shown.clone()
                };
                if let Some(active) = self.import_slot().as_mut() {
                    active.shown = shown.clone();
                }
                self.announce_import(Some(shown));
            }),
        );
        match result {
            Ok(id) => {
                info!("Meeting {id} imported from an audio file");
                self.queue.enqueue(id);
                self.updated(id);
                Ok(Some(id))
            }
            Err(Stopped::Cancelled) => {
                info!("Audio import cancelled");
                Ok(None)
            }
            Err(Stopped::Failed(error)) => {
                warn!("Audio import failed: {error}");
                Err(error)
            }
        }
    }

    pub fn is_recording(&self) -> bool {
        self.runtime() != MeetingRuntime::Idle
    }

    pub fn start(&self) -> Result<(), String> {
        self.start_with(&mut self.coordinator())
    }

    fn start_with(&self, coordinator: &mut AppCoordinator) -> Result<(), String> {
        let settings = get_settings(&self.app);
        if !settings.meetings_enabled {
            return Err("Meetings are turned off".to_string());
        }
        let id = coordinator
            .start(settings.meeting_capture_mode)
            .map_err(|error| error.to_string())?;
        // Opening the devices takes a moment. Meetings turned off meanwhile
        // found nothing recording to stop, and left no way to stop this one.
        if !get_settings(&self.app).meetings_enabled {
            let _ = coordinator.discard(id);
            return Err("Meetings were turned off while the meeting started".to_string());
        }
        info!("Meeting {id} started");
        self.spawn_ticker(id);
        // The speaker models download while the meeting records, so they are
        // ready when it stops. Intel Macs tell no speakers apart.
        if helper::support() == AnalysisSupport::Available {
            self.models.download_unless_declined();
        }
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        Self::stop_with(&mut self.coordinator())
    }

    fn stop_with(coordinator: &mut AppCoordinator) -> Result<(), String> {
        let id = coordinator.stop().map_err(|error| error.to_string())?;
        info!("Meeting {id} recorded");
        Ok(())
    }

    pub fn discard(&self, id: i64) -> Result<(), String> {
        self.coordinator()
            .discard(id)
            .map_err(|error| error.to_string())?;
        self.forget_live(id);
        Ok(())
    }

    fn forget_live(&self, id: i64) {
        if let Some(live) = self.app.try_state::<Arc<live::Live>>() {
            live.forget(id);
        }
    }

    /// The live transcript of meeting `id`, or the draft left of it.
    pub fn live_transcript(&self, id: i64) -> Option<live::LiveTranscript> {
        let live = self.app.try_state::<Arc<live::Live>>()?;
        live::transcript_of(&live, &paths::meeting_dir(&self.root, id), id)
    }

    /// Decides under the lock, so two quick presses never start two meetings.
    pub fn toggle(&self) -> Result<(), String> {
        let mut coordinator = self.coordinator();
        if coordinator.runtime() == MeetingRuntime::Idle {
            self.start_with(&mut coordinator)
        } else {
            Self::stop_with(&mut coordinator)
        }
    }

    pub fn list(&self, cursor: Option<i64>, limit: usize) -> Result<MeetingsPage, String> {
        self.store
            .list(cursor, limit)
            .map_err(|error| error.to_string())
    }

    pub fn search(&self, query: &str) -> Result<Vec<MeetingHit>, String> {
        self.store
            .search(query, crate::search::SEARCH_LIMIT)
            .map_err(|error| crate::search::search_failed("Meeting", &error))
    }

    /// Moves the meeting to the Trash, with its notes, and forgets it.
    pub fn delete(&self, id: i64) -> Result<(), String> {
        self.queue.cancel(id);
        let Some(meeting) = self.store.get(id).map_err(|error| error.to_string())? else {
            self.forget_live(id);
            return Ok(());
        };
        let notes = trash::Notes::of(&meeting)
            .ok_or_else(|| format!("Meeting {id} has an invalid start time"))?;
        self.coordinator()
            .delete(id, &notes, &trash::SystemTrash)
            .map_err(|error| error.to_string())?;
        self.forget_live(id);
        self.updated(id);
        Ok(())
    }

    pub fn detail(&self, id: i64) -> Result<MeetingDetail, String> {
        let mut meeting = self
            .store
            .get(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Meeting {id} does not exist"))?;
        meeting.held_until_ms = self.queue.held_for(id).map(|left| {
            chrono::Utc::now().timestamp_millis() + i64::try_from(left.as_millis()).unwrap_or(0)
        });
        meeting.waiting_for_connection = self.queue.offline_for(id);
        Ok(meeting)
    }

    pub fn announcement_name(&self, id: i64) -> Result<Option<announce::MeetingName>, String> {
        self.store
            .announcement_name(id)
            .map_err(|error| error.to_string())
    }

    pub(super) fn updated(&self, id: i64) {
        AppEvents(self.app.clone()).updated(id);
    }

    /// An empty title gives the meeting back its default name.
    pub fn rename(&self, id: i64, title: &str) -> Result<(), String> {
        let title = rename::title(title).map_err(|error| error.to_string())?;
        self.store
            .rename(id, title.as_deref())
            .map_err(|error| error.to_string())?;
        self.updated(id);
        Ok(())
    }

    pub fn rename_speaker(&self, id: i64, key: &str, name: &str) -> Result<(), String> {
        let name = rename::speaker_name(name).map_err(|error| error.to_string())?;
        self.store
            .rename_speaker(id, key, &name)
            .map_err(|error| error.to_string())?;
        self.updated(id);
        Ok(())
    }

    /// Processes an interrupted meeting, or the parts of a failed one that
    /// are missing. It shows as waiting at once, even while another meeting
    /// is processing.
    pub fn retry(&self, id: i64) {
        match self.store.wait_for_retry(id) {
            Ok(true) => self.updated(id),
            Ok(false) => {}
            Err(error) => error!("Meeting {id}: cannot mark it waiting: {error}"),
        }
        self.queue.enqueue_asked(id);
    }

    /// Tells the speakers of a finished meeting apart again, with the models
    /// as they are now. Codex is not asked again for parts it transcribed.
    pub fn reanalyze(&self, id: i64) -> Result<(), String> {
        if self.detail(id)?.audio == store::AudioState::Deleted {
            return Err(format!("Meeting {id} has no audio left"));
        }
        // The daily retention may delete the audio after the check above.
        if !self.store.reopen(id).map_err(|error| error.to_string())? {
            return Err(format!("Meeting {id} is not finished or has no audio left"));
        }
        // A summary still being written is of the analysis that goes.
        self.queue.cancel(id);
        // Nothing processes the meeting until it is queued below.
        let analysis = paths::meeting_dir(&self.root, id).join(analysis::ANALYSIS_FILE);
        match std::fs::remove_file(analysis) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => error!("Meeting {id}: cannot reset the analysis: {error}"),
        }
        self.updated(id);
        self.queue.enqueue_asked(id);
        Ok(())
    }

    /// Asks Codex again for a summary that failed.
    pub fn retry_summary(&self, id: i64) -> Result<(), String> {
        if !self
            .store
            .retry_summary(id)
            .map_err(|error| error.to_string())?
        {
            return Err(format!("Meeting {id} has no failed summary"));
        }
        self.updated(id);
        self.queue.enqueue_asked(id);
        Ok(())
    }

    /// Moves a finished meeting's audio to the Trash and deletes the analysis
    /// made from it; the transcript and the summary stay.
    pub fn delete_audio(&self, id: i64) -> Result<(), String> {
        let meeting = self.detail(id)?;
        let notes = trash::Notes::of(&meeting)
            .ok_or_else(|| format!("Meeting {id} has an invalid start time"))?;
        let result = trash_audio(&self.store, &self.root, id, &notes, &trash::SystemTrash);
        self.updated(id);
        result
    }

    /// Deletes the audio the retention the user chose has kept long enough.
    /// A meeting queued or processing keeps it until it is done.
    pub(super) fn apply_audio_retention(&self) {
        let retention = get_settings(&self.app).meeting_audio_retention;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let deleted = sweep_audio(
            &self.store,
            &self.root,
            retention,
            now_ms,
            &self.queue.busy(),
        );
        if !deleted.is_empty() {
            info!("Deleted the audio of meeting(s) {deleted:?}, kept as long as chosen");
        }
        deleted.into_iter().for_each(|id| self.updated(id));
    }

    /// Transcribes a finished meeting again from its audio, and writes its
    /// summary anew. A title the user gave stays.
    pub fn retranscribe(&self, id: i64) -> Result<(), String> {
        if !self
            .store
            .retranscribe(id)
            .map_err(|error| error.to_string())?
        {
            return Err(format!("Meeting {id} cannot be transcribed again"));
        }
        // A summary still being written is of the transcript that goes.
        self.queue.cancel(id);
        self.updated(id);
        self.queue.enqueue_asked(id);
        Ok(())
    }

    /// Lets processing run again and queues the meetings a quit (or turning
    /// meetings off) left mid-way.
    pub fn resume_processing(&self) {
        self.queue.resume();
        queue_waiting(&self.queue, &self.store);
    }

    /// The summary or the transcript as Markdown, and the file name to save
    /// it under.
    pub fn markdown(&self, id: i64, kind: MeetingMarkdownKind) -> Result<(String, String), String> {
        let meeting = self.detail(id)?;
        let started_at = export::local_start(meeting.started_at_ms)
            .ok_or_else(|| format!("Meeting {id} has an invalid start time"))?;
        let transcript = export::Transcript {
            title: meeting.title.as_deref(),
            started_at,
            duration_ms: meeting.duration_ms,
            speakers: &meeting.speakers,
            turns: &meeting.turns,
        };
        let markdown = match kind {
            MeetingMarkdownKind::Transcript => export::transcript_markdown(&transcript),
            MeetingMarkdownKind::Summary => export::summary_markdown(
                &transcript,
                meeting
                    .summary
                    .as_deref()
                    .ok_or_else(|| format!("Meeting {id} has no summary"))?,
            ),
        };
        Ok((
            markdown,
            export::file_name(&started_at, meeting.title.as_deref()),
        ))
    }

    /// Keeps a meeting in progress when the app quits or meetings are turned
    /// off: its tracks are closed and it waits for processing, which resumes
    /// at the next launch or when meetings are turned back on.
    pub fn shutdown(&self) {
        if self.is_recording() {
            if let Err(error) = self.stop() {
                error!("Failed to finish the meeting on quit: {error}");
            }
        }
        // A models download goes on: dictation shares the models.
        self.queue.pause();
    }

    /// Drives the cap and write-failure checks of meeting `id` until it ends.
    fn spawn_ticker(&self, id: i64) {
        let app = self.app.clone();
        let spawned = std::thread::Builder::new()
            .name("meeting-ticker".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let Some(manager) = app.try_state::<Arc<MeetingsManager>>() else {
                    return;
                };
                if !manager.coordinator().tick(id) {
                    return;
                }
            });
        if let Err(error) = spawned {
            error!("Failed to start the meeting timer: {error}");
        }
    }
}
