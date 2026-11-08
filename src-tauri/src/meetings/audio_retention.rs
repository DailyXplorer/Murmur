use super::*;

/// Erases finished meeting `id`'s audio and the analysis made from it. The
/// audio retention uses this: it exists to free space.
pub(super) fn delete_audio(store: &MeetingsStore, root: &Path, id: i64) -> Result<(), String> {
    delete_audio_with(store, id, || {
        paths::remove_meeting_files(root, id, &paths::AUDIO_FILES)
            .and_then(|()| paths::remove_meeting_files(root, id, &[analysis::ANALYSIS_FILE]))
    })
}

/// A person's "Delete audio": the audio goes to the Trash, the analysis for
/// good. The Trash comes first, the one step that cannot be undone by a
/// retry: files already in the Trash are not looked for again.
pub(super) fn trash_audio(
    store: &MeetingsStore,
    root: &Path,
    id: i64,
    notes: &trash::Notes,
    bin: &dyn trash::Trash,
) -> Result<(), String> {
    delete_audio_with(store, id, || {
        trash::trash_audio(root, id, notes, bin)
            .and_then(|()| paths::remove_meeting_files(root, id, &[analysis::ANALYSIS_FILE]))
    })
}

fn delete_audio_with(
    store: &MeetingsStore,
    id: i64,
    remove_files: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), String> {
    let deleted = store
        .delete_audio(id, remove_files)
        .map_err(|error| format!("Cannot delete the meeting audio: {error}"))?;
    if !deleted {
        return Err(format!("Meeting {id} is not finished"));
    }
    Ok(())
}

/// Deletes the audio `retention` has kept long enough at `now_ms`, except
/// that of the `busy` meetings, and returns whose went.
pub(super) fn sweep_audio(
    store: &MeetingsStore,
    root: &Path,
    retention: MeetingAudioRetention,
    now_ms: i64,
    busy: &HashSet<i64>,
) -> Vec<i64> {
    let meetings = match store.with_audio() {
        Ok(meetings) => meetings,
        Err(error) => {
            error!("Failed to list the meetings that have audio: {error}");
            return Vec::new();
        }
    };
    retention::audio_to_delete(&meetings, now_ms, retention)
        .into_iter()
        .filter(|id| !busy.contains(id))
        .filter(|id| match delete_audio(store, root, *id) {
            Ok(()) => true,
            Err(error) => {
                error!("Meeting {id}: {error}");
                false
            }
        })
        .collect()
}

/// Whether the audio retention runs at `now`, last run at `last`: at launch,
/// then once a day, while `settings` keep the audio for a limited time.
/// Meetings turned off still lose their audio when the user chose.
pub(super) fn audio_retention_due(
    settings: &AppSettings,
    last: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    const DAY: Duration = Duration::from_secs(24 * 3600);
    settings.meeting_audio_retention != MeetingAudioRetention::Keep
        && last.is_none_or(|last| now.duration_since(last).map_or(true, |since| since >= DAY))
}

/// Applies the audio retention when it is due. The hourly look reads the
/// wall clock, which goes on while the Mac sleeps.
pub(super) fn spawn_audio_retention(app: &AppHandle) {
    const LOOK: Duration = Duration::from_secs(3600);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last: Option<SystemTime> = None;
        loop {
            if audio_retention_due(&get_settings(&app), last, SystemTime::now()) {
                if let Some(manager) = app.try_state::<Arc<MeetingsManager>>() {
                    let manager = Arc::clone(&manager);
                    let swept = tauri::async_runtime::spawn_blocking(move || {
                        manager.apply_audio_retention()
                    });
                    if let Err(error) = swept.await {
                        error!("Meeting audio retention failed: {error}");
                    }
                    last = Some(SystemTime::now());
                }
            }
            tokio::time::sleep(LOOK).await;
        }
    });
}
