use super::download::test_server::{self, Reply, Request};
use super::*;
use sha2::Digest;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct FakeHost {
    wanted: Mutex<Vec<bool>>,
    free: Option<u64>,
    prepared: AtomicUsize,
    prepare_fails: bool,
    notified: Mutex<Vec<LocalModelState>>,
}

impl Host for FakeHost {
    fn notify(&self, status: LocalModelStatus) {
        self.notified.lock().unwrap().push(status.state);
    }
    fn remember_wanted(&self, wanted: bool) {
        self.wanted.lock().unwrap().push(wanted);
    }
    fn free_bytes(&self, _: &Path) -> Option<u64> {
        self.free
    }
    fn prepare(&self, _: PathBuf, _: ProcessingOperation) -> PrepareFuture {
        self.prepared.fetch_add(1, Ordering::SeqCst);
        let fails = self.prepare_fails;
        Box::pin(async move {
            if fails {
                Prepared::Failed
            } else {
                Prepared::Done
            }
        })
    }
}

/// Two small sources whose files hold `contents`, the second one pinned by
/// FluidAudio, and with `fixture_with_optional` a third, optional one.
struct Fixture {
    sources: &'static [Source],
    contents: Vec<(&'static str, Vec<u8>)>,
}

fn fixture() -> Fixture {
    build_fixture(false)
}

fn fixture_with_optional() -> Fixture {
    build_fixture(true)
}

const CTC_WEIGHTS: &str = "C.mlmodelc/weights/weight.bin";

fn build_fixture(with_optional: bool) -> Fixture {
    let mut contents: Vec<(&'static str, Vec<u8>)> = vec![
        ("A.mlmodelc/weights/weight.bin", vec![1; 6_000]),
        ("vocab.json", b"{\"0\":\"a\"}".to_vec()),
        ("plda.json", vec![7; 2_000]),
    ];
    if with_optional {
        contents.push((CTC_WEIGHTS, vec![9; 3_000]));
    }
    let file = |index: usize| -> ModelFile {
        let (path, bytes) = &contents[index];
        ModelFile {
            path,
            size: bytes.len() as u64,
            sha256: Box::leak(format!("{:x}", sha2::Sha256::digest(bytes)).into_boxed_str()),
        }
    };
    let asr: &'static [ModelFile] = Box::leak(vec![file(0), file(1)].into_boxed_slice());
    let diarizer: &'static [ModelFile] = Box::leak(vec![file(2)].into_boxed_slice());
    let mut sources = vec![
        Source {
            repo: "test/asr",
            commit: "1111111111111111111111111111111111111111",
            folder: "asr",
            fluid_revision_marker: false,
            optional: false,
            files: asr,
        },
        Source {
            repo: "test/diarizer",
            commit: "2222222222222222222222222222222222222222",
            folder: "diarizer",
            fluid_revision_marker: true,
            optional: false,
            files: diarizer,
        },
    ];
    if with_optional {
        sources.push(Source {
            repo: "test/ctc",
            commit: "3333333333333333333333333333333333333333",
            folder: "ctc",
            fluid_revision_marker: false,
            optional: true,
            files: Box::leak(vec![file(3)].into_boxed_slice()),
        });
    }
    Fixture {
        sources: Box::leak(sources.into_boxed_slice()),
        contents,
    }
}

impl Fixture {
    fn body_of(&self, path: &str) -> Option<Vec<u8>> {
        self.contents
            .iter()
            .find(|(file, _)| path.ends_with(&format!("/{file}")))
            .map(|(_, bytes)| bytes.clone())
    }

    /// Serves every file, counting requests.
    fn serving(&self) -> test_server::Script {
        let contents = self.contents.clone();
        Arc::new(move |request: &Request, _| {
            match contents
                .iter()
                .find(|(file, _)| request.path.ends_with(&format!("/{file}")))
            {
                Some((_, bytes)) => Reply::Serve(bytes.clone()),
                None => Reply::NotFound,
            }
        })
    }

    /// Lays every file out as a finished download would.
    fn write_all(&self, dir: &Path) {
        self.write(dir, |_| true);
    }

    /// Lays out what a version without the optional source downloaded, with
    /// the ready marker it wrote.
    fn write_ready_without_optional(&self, dir: &Path) {
        self.write(dir, |source| !source.optional);
        let earlier: Vec<&Source> = self.sources.iter().filter(|s| !s.optional).collect();
        std::fs::write(dir.join(READY_MARKER), manifest::id(earlier)).unwrap();
    }

    fn write(&self, dir: &Path, taken: impl Fn(&Source) -> bool) {
        for source in self.sources.iter().filter(|source| taken(source)) {
            for file in source.files {
                let path = dir.join(source.folder).join(file.path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, self.body_of(&format!("/{}", file.path)).unwrap()).unwrap();
            }
        }
    }
}

fn models(dir: &Path, fixture: &Fixture, base: &str, host: &Arc<FakeHost>) -> Arc<LocalModels> {
    let host: Arc<dyn Host> = Arc::clone(host) as Arc<dyn Host>;
    LocalModels::new(
        dir.to_path_buf(),
        fixture.sources,
        AnalysisSupport::Available,
        base.to_string(),
        host,
    )
}

async fn settled(models: &LocalModels) -> LocalModelState {
    models.settled().await
}

#[test]
fn a_download_fetches_every_file_prepares_once_and_survives_a_restart() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, seen) = test_server::start(fixture.serving()).await;
        let models = models(&path, &fixture, &base, &host);
        assert_eq!(models.status().state, LocalModelState::Missing);
        assert_eq!(models.status().size_bytes, 8_009);
        models.download();
        assert_eq!(settled(&models).await, LocalModelState::Ready);
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert_eq!(
            seen.lock().unwrap()[0].path,
            "/test/asr/resolve/1111111111111111111111111111111111111111/A.mlmodelc/weights/weight.bin"
        );
        assert_eq!(host.prepared.load(Ordering::SeqCst), 1);
        assert_eq!(*host.wanted.lock().unwrap(), [true]);
        assert_eq!(
            std::fs::read_to_string(path.join("diarizer/.fluidaudio-revision")).unwrap(),
            "2222222222222222222222222222222222222222\n"
        );
        assert!(!path.join("asr/.fluidaudio-revision").exists());

        let after_restart = models_after_restart(&path, &fixture, &host);
        assert!(after_restart.is_ready());
    });
}

fn models_after_restart(path: &Path, fixture: &Fixture, host: &Arc<FakeHost>) -> Arc<LocalModels> {
    models(path, fixture, "http://127.0.0.1:9", host)
}

#[test]
fn models_from_an_earlier_version_that_match_download_nothing() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    fixture.write_all(&path);
    // The marker FluidAudio-era Murmur wrote: empty.
    std::fs::write(path.join(READY_MARKER), b"").unwrap();
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, seen) = test_server::start(fixture.serving()).await;
        let models = models(&path, &fixture, &base, &host);
        assert_eq!(models.status().state, LocalModelState::Preparing);
        models.resume_on_launch(false);
        assert_eq!(settled(&models).await, LocalModelState::Ready);
        assert!(seen.lock().unwrap().is_empty());
        assert_eq!(host.prepared.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read_to_string(path.join(READY_MARKER)).unwrap(),
            manifest::id(fixture.sources)
        );
    });
}

#[test]
fn an_earlier_folder_with_one_wrong_file_downloads_only_that_file() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    fixture.write_all(&path);
    std::fs::write(path.join("asr/vocab.json"), b"{\"0\":\"b\"}").unwrap();
    std::fs::write(path.join(READY_MARKER), b"").unwrap();
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, seen) = test_server::start(fixture.serving()).await;
        let models = models(&path, &fixture, &base, &host);
        models.resume_on_launch(false);
        assert_eq!(settled(&models).await, LocalModelState::Ready);
        let paths: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.path.clone())
            .collect();
        assert_eq!(
            paths,
            ["/test/asr/resolve/1111111111111111111111111111111111111111/vocab.json"]
        );
        assert_eq!(
            std::fs::read(path.join("asr/vocab.json")).unwrap(),
            b"{\"0\":\"a\"}"
        );
        assert_eq!(host.prepared.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn a_cancelled_check_of_an_earlier_folder_stays_cancelled_after_a_restart() {
    let fixture = fixture();
    let vocab = fixture.contents[1].1.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    fixture.write_all(&path);
    std::fs::write(path.join("asr/vocab.json"), b"{\"0\":\"b\"}").unwrap();
    std::fs::write(path.join(READY_MARKER), b"").unwrap();
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, _) =
            test_server::start(Arc::new(move |_, _| Reply::HangAfter(vocab.clone(), 0))).await;
        let checking = models(&path, &fixture, &base, &host);
        checking.resume_on_launch(false);
        checking
            .state
            .subscribe()
            .wait_for(|state| matches!(state, LocalModelState::Downloading { .. }))
            .await
            .unwrap();
        checking.cancel();
        assert_eq!(settled(&checking).await, LocalModelState::Missing);

        let (base, seen) = test_server::start(fixture.serving()).await;
        let after_restart = models(&path, &fixture, &base, &host);
        assert_eq!(after_restart.status().state, LocalModelState::Missing);
        after_restart.resume_on_launch(false);
        assert_eq!(settled(&after_restart).await, LocalModelState::Missing);
        assert!(seen.lock().unwrap().is_empty());
    });
}

#[test]
fn a_download_the_user_wanted_resumes_at_the_next_launch() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let weights = path.join("asr/A.mlmodelc/weights/weight.bin");
    std::fs::create_dir_all(weights.parent().unwrap()).unwrap();
    std::fs::write(download::part_path(&weights), vec![1; 2_500]).unwrap();
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, seen) = test_server::start(fixture.serving()).await;

        let not_wanted = models(&path, &fixture, &base, &host);
        not_wanted.resume_on_launch(false);
        assert_eq!(not_wanted.status().state, LocalModelState::Missing);
        assert!(seen.lock().unwrap().is_empty());

        let wanted = models(&path, &fixture, &base, &host);
        wanted.resume_on_launch(true);
        assert_eq!(settled(&wanted).await, LocalModelState::Ready);
        assert_eq!(seen.lock().unwrap()[0].range_from, Some(2_500));
        assert_eq!(std::fs::read(&weights).unwrap(), vec![1; 6_000]);
    });
}

#[test]
fn delete_is_refused_while_downloading_and_a_cancel_is_remembered() {
    let fixture = fixture();
    let weights = fixture.contents[0].1.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, _) = test_server::start(Arc::new(move |_, _| {
            Reply::HangAfter(weights.clone(), 1_000)
        }))
        .await;
        let models = models(&path, &fixture, &base, &host);
        models.download();
        assert!(models.delete().is_err());
        models.cancel();
        assert_eq!(settled(&models).await, LocalModelState::Missing);
        assert!(models.declined());
        assert_eq!(*host.wanted.lock().unwrap(), [true, false]);

        models.delete().unwrap();
        assert!(!path.exists());
        assert_eq!(models.status().state, LocalModelState::Missing);
    });
}

#[test]
fn a_full_disk_is_reported_before_any_download() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(FakeHost {
        free: Some(5_000),
        ..FakeHost::default()
    });
    tauri::async_runtime::block_on(async {
        let (base, seen) = test_server::start(fixture.serving()).await;
        let models = models(&dir.path().join("models"), &fixture, &base, &host);
        models.download();
        assert_eq!(
            settled(&models).await,
            LocalModelState::Failed {
                reason: DownloadFailure::NotEnoughSpace {
                    needed_bytes: 8_809,
                    free_bytes: 5_000
                }
            }
        );
        assert!(seen.lock().unwrap().is_empty());
    });
}

#[test]
fn models_that_will_not_load_are_not_marked_ready() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let host = Arc::new(FakeHost {
        prepare_fails: true,
        ..FakeHost::default()
    });
    tauri::async_runtime::block_on(async {
        let (base, _) = test_server::start(fixture.serving()).await;
        let models = models(&path, &fixture, &base, &host);
        models.download();
        assert_eq!(
            settled(&models).await,
            LocalModelState::Failed {
                reason: DownloadFailure::PrepareFailed
            }
        );
        assert!(!path.join(READY_MARKER).exists());
    });
}

#[test]
fn models_that_will_not_load_report_what_they_keep_until_deleted() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let host = Arc::new(FakeHost {
        prepare_fails: true,
        ..FakeHost::default()
    });
    tauri::async_runtime::block_on(async {
        let (base, _) = test_server::start(fixture.serving()).await;
        let models = models(&path, &fixture, &base, &host);
        models.download();
        settled(&models).await;
        // The three files and FluidAudio's 41-byte revision marker.
        assert_eq!(models.status().kept_bytes, 8_050);

        models.delete().unwrap();
        assert_eq!(models.status().state, LocalModelState::Missing);
        assert_eq!(models.status().kept_bytes, 0);
    });
}

#[test]
fn a_cancelled_download_reports_its_partial_files() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let weights = path.join("asr/A.mlmodelc/weights/weight.bin");
    std::fs::create_dir_all(weights.parent().unwrap()).unwrap();
    std::fs::write(download::part_path(&weights), vec![1; 2_500]).unwrap();
    let cancelled = models_after_restart(&path, &fixture, &Arc::new(FakeHost::default()));
    assert_eq!(cancelled.status().state, LocalModelState::Missing);
    assert_eq!(cancelled.status().kept_bytes, 2_500);
}

#[test]
fn unsupported_macs_never_download() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let host: Arc<dyn Host> = Arc::new(FakeHost::default());
    let old = LocalModels::new(
        dir.path().to_path_buf(),
        fixture.sources,
        AnalysisSupport::OsTooOld,
        "http://127.0.0.1:9".into(),
        host,
    );
    old.download();
    assert_eq!(
        old.status().state,
        LocalModelState::Unsupported {
            support: AnalysisSupport::OsTooOld
        }
    );
}

#[test]
fn models_the_helper_found_missing_are_downloaded_again() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    fixture.write_all(&path);
    std::fs::write(path.join(READY_MARKER), manifest::id(fixture.sources)).unwrap();
    let ready = models_after_restart(&path, &fixture, &Arc::new(FakeHost::default()));
    assert!(ready.is_ready());
    ready.invalidate();
    assert_eq!(ready.status().state, LocalModelState::Missing);
    assert!(!path.join(READY_MARKER).exists());
}

/// Ready models in `path`, for the engine's tests, and one of their files,
/// which stands in for the model the engine loads.
pub(crate) fn ready_models(path: &Path) -> (Arc<LocalModels>, PathBuf) {
    let fixture = fixture();
    fixture.write_all(path);
    std::fs::write(path.join(READY_MARKER), manifest::id(fixture.sources)).unwrap();
    let ready = models_after_restart(path, &fixture, &Arc::new(FakeHost::default()));
    assert!(ready.is_ready());
    let model = path
        .join(fixture.sources[0].folder)
        .join(fixture.sources[0].files[0].path);
    (ready, model)
}

/// Keeps a file of the folder from being deleted until dropped, as Windows
/// does with a file another program holds open.
struct Undeletable {
    #[cfg(target_os = "windows")]
    _held: std::fs::File,
    #[cfg(not(target_os = "windows"))]
    folder: PathBuf,
}

impl Undeletable {
    fn new(file: &Path) -> Self {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 1;
            Self {
                _held: std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ)
                    .open(file)
                    .unwrap(),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::PermissionsExt;
            let folder = file.parent().unwrap().to_path_buf();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
            Self { folder }
        }
    }
}

#[cfg(not(target_os = "windows"))]
impl Drop for Undeletable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.folder, std::fs::Permissions::from_mode(0o755));
    }
}

/// A delete stopped halfway, as on Windows by a file in use, says so and
/// leaves models that are no longer ready, now and after a restart, so
/// dictation stops counting on them. Deleting again finishes the job.
#[test]
fn a_delete_stopped_halfway_is_reported_and_leaves_no_ready_models() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let (ready, model) = ready_models(&path);
    let held = Undeletable::new(&model);

    assert!(ready.delete().is_err());
    assert_eq!(ready.status().state, LocalModelState::Missing);
    assert!(ready.status().kept_bytes > 0, "nothing left to delete");
    let host = Arc::new(FakeHost::default());
    assert!(!models_after_restart(&path, &fixture, &host).is_ready());

    drop(held);
    ready.delete().unwrap();
    assert!(!path.exists());
}

#[test]
fn delete_removes_everything_and_reports_missing() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    fixture.write_all(&path);
    std::fs::write(path.join(READY_MARKER), manifest::id(fixture.sources)).unwrap();
    let ready = models_after_restart(&path, &fixture, &Arc::new(FakeHost::default()));
    assert!(ready.is_ready());
    ready.delete().unwrap();
    assert!(!path.exists());
    assert_eq!(ready.status().state, LocalModelState::Missing);
}

/// A meeting that starts while the user deletes a partial download starts
/// no download into the folder going away.
#[test]
fn a_meeting_start_during_a_delete_downloads_nothing() {
    let fixture = fixture();
    let weights = fixture.contents[0].1.clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let partial = path.join("partial");
    std::fs::create_dir_all(&partial).unwrap();
    for index in 0..3_000 {
        std::fs::write(partial.join(index.to_string()), b"x").unwrap();
    }
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, _) = test_server::start(Arc::new(move |_, _| {
            Reply::HangAfter(weights.clone(), 1_000)
        }))
        .await;
        let models = models(&path, &fixture, &base, &host);
        let meeting = {
            let models = Arc::clone(&models);
            let partial = partial.clone();
            std::thread::spawn(move || {
                // Once the delete is removing files.
                while std::fs::read_dir(&partial).is_ok_and(|entries| entries.count() == 3_000) {}
                models.download_unless_declined();
            })
        };
        let deleted = models.delete();
        meeting.join().unwrap();
        deleted.unwrap();
        assert!(models.declined());
        assert!(models.download_slot().is_none(), "a download started");
        assert_eq!(models.status().state, LocalModelState::Missing);
    });
}

/// Ready models whose optional source is still coming, from a server that
/// hangs partway through it.
fn ready_while_the_optional_source_hangs(
    path: &Path,
    fixture: &Fixture,
    base: &str,
) -> Arc<LocalModels> {
    fixture.write_ready_without_optional(path);
    let ready = models(path, fixture, base, &Arc::new(FakeHost::default()));
    ready.resume_on_launch(false);
    assert!(ready.is_ready());
    ready
}

#[test]
fn delete_stops_the_optional_fetch_of_ready_models() {
    let fixture = fixture_with_optional();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    tauri::async_runtime::block_on(async {
        let (base, _) = test_server::start(Arc::new(|_: &Request, _| {
            Reply::HangAfter(vec![9; 3_000], 1_000)
        }))
        .await;
        let ready = ready_while_the_optional_source_hangs(&path, &fixture, &base);
        ready.delete().unwrap();
        assert!(!path.exists());
        assert_eq!(ready.status().state, LocalModelState::Missing);
        ready.fetched().await;
        assert!(!path.exists(), "written after the delete");
    });
}

#[test]
fn models_found_missing_download_again_during_the_optional_fetch() {
    let fixture = fixture_with_optional();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    tauri::async_runtime::block_on(async {
        let (base, _) = test_server::start(Arc::new(|_: &Request, _| {
            Reply::HangAfter(vec![9; 3_000], 1_000)
        }))
        .await;
        let ready = ready_while_the_optional_source_hangs(&path, &fixture, &base);
        ready.invalidate();
        ready.download();
        assert_eq!(settled(&ready).await, LocalModelState::Ready);
    });
}

fn paths(seen: &Mutex<Vec<Request>>) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|r| r.path.clone())
        .collect()
}

const CTC_URL: &str =
    "/test/ctc/resolve/3333333333333333333333333333333333333333/C.mlmodelc/weights/weight.bin";

#[test]
fn models_ready_before_the_optional_source_stay_ready_and_fetch_it_quietly() {
    let fixture = fixture_with_optional();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    fixture.write_ready_without_optional(&path);
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, seen) = test_server::start(fixture.serving()).await;
        let ready = models(&path, &fixture, &base, &host);
        assert!(ready.is_ready());
        ready.resume_on_launch(false);
        assert!(ready.is_ready());
        ready.fetched().await;
        assert!(ready.is_ready());
        assert!(host.notified.lock().unwrap().is_empty());
        assert_eq!(paths(&seen), [CTC_URL]);
        assert_eq!(
            std::fs::read(path.join("ctc").join(CTC_WEIGHTS)).unwrap(),
            vec![9; 3_000]
        );
        // Loaded once, so Core ML compiles the new model before a dictation.
        assert_eq!(host.prepared.load(Ordering::SeqCst), 1);

        let after_restart = models(&path, &fixture, &base, &host);
        after_restart.resume_on_launch(false);
        after_restart.fetched().await;
        assert!(after_restart.is_ready());
        assert_eq!(seen.lock().unwrap().len(), 1, "fetched again");
        assert_eq!(host.prepared.load(Ordering::SeqCst), 1, "prepared again");
    });
}

#[test]
fn a_new_download_is_ready_before_the_optional_source_and_then_fetches_it() {
    let fixture = fixture_with_optional();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, seen) = test_server::start(fixture.serving()).await;
        let models = models(&path, &fixture, &base, &host);
        assert_eq!(models.status().size_bytes, 11_009);
        models.download();
        assert_eq!(settled(&models).await, LocalModelState::Ready);
        models.fetched().await;
        assert!(models.is_ready());
        assert_eq!(paths(&seen).last().map(String::as_str), Some(CTC_URL));
        assert_eq!(seen.lock().unwrap().len(), 4);
        assert!(path.join("ctc").join(CTC_WEIGHTS).exists());
        assert_eq!(host.prepared.load(Ordering::SeqCst), 2);
        // Progress counts what dictation waits for.
        assert!(host
            .notified
            .lock()
            .unwrap()
            .iter()
            .all(|state| match state {
                LocalModelState::Downloading { total_bytes, .. } => *total_bytes == 8_009,
                _ => true,
            }));
        assert_eq!(
            host.notified.lock().unwrap().last(),
            Some(&LocalModelState::Ready)
        );
    });
}

#[test]
fn a_corrupt_optional_file_never_blocks_ready() {
    let fixture = fixture_with_optional();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models");
    fixture.write_ready_without_optional(&path);
    let ctc = path.join("ctc").join(CTC_WEIGHTS);
    std::fs::create_dir_all(ctc.parent().unwrap()).unwrap();
    std::fs::write(&ctc, vec![8; 3_000]).unwrap();
    let host = Arc::new(FakeHost::default());
    tauri::async_runtime::block_on(async {
        let (base, seen) =
            test_server::start(Arc::new(|_: &Request, _| Reply::Serve(vec![8; 3_000]))).await;
        let ready = models(&path, &fixture, &base, &host);
        ready.resume_on_launch(false);
        ready.fetched().await;
        assert!(ready.is_ready());
        assert!(host.notified.lock().unwrap().is_empty());
        assert!(!seen.lock().unwrap().is_empty());
        assert!(!ctc.exists(), "the corrupt file was kept");

        let (base, _) = test_server::start(fixture.serving()).await;
        let after_restart = models(&path, &fixture, &base, &host);
        after_restart.resume_on_launch(false);
        after_restart.fetched().await;
        assert!(after_restart.is_ready());
        assert_eq!(std::fs::read(&ctc).unwrap(), vec![9; 3_000]);
    });
}

/// After a pin change the marker of the earlier files stays until the new
/// ones are prepared: the helper must not count on files being replaced.
#[test]
fn an_optional_marker_of_another_pin_is_not_ready() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join(OPTIONAL_MARKER);
    std::fs::write(
        &marker,
        "earlier/ctc@1111111111111111111111111111111111111111",
    )
    .unwrap();
    assert!(!optional_ready(dir.path()));

    let optional = manifest::this_mac().iter().filter(|source| source.optional);
    std::fs::write(&marker, manifest::id(optional)).unwrap();
    assert!(optional_ready(dir.path()));
}

#[test]
fn waiting_returns_at_once_when_nothing_downloads() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let missing = models_after_restart(dir.path(), &fixture, &Arc::new(FakeHost::default()));
    let operation = ProcessingOperation::new(OperationId(1));
    assert!(!tauri::async_runtime::block_on(
        missing.wait_ready(&operation)
    ));
}
