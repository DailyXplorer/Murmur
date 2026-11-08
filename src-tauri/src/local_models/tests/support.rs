use super::*;

pub(super) fn fixture() -> Fixture {
    build_fixture(false)
}

pub(super) fn fixture_with_optional() -> Fixture {
    build_fixture(true)
}

pub(super) const CTC_WEIGHTS: &str = "C.mlmodelc/weights/weight.bin";

pub(super) fn build_fixture(with_optional: bool) -> Fixture {
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
    pub(super) fn serving(&self) -> test_server::Script {
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
    pub(super) fn write_all(&self, dir: &Path) {
        self.write(dir, |_| true);
    }

    /// Lays out what a version without the optional source downloaded, with
    /// the ready marker it wrote.
    pub(super) fn write_ready_without_optional(&self, dir: &Path) {
        self.write(dir, |source| !source.optional);
        let earlier: Vec<&Source> = self.sources.iter().filter(|s| !s.optional).collect();
        std::fs::write(dir.join(READY_MARKER), manifest::id(earlier)).unwrap();
    }

    pub(super) fn write(&self, dir: &Path, taken: impl Fn(&Source) -> bool) {
        for source in self.sources.iter().filter(|source| taken(source)) {
            for file in source.files {
                let path = dir.join(source.folder).join(file.path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, self.body_of(&format!("/{}", file.path)).unwrap()).unwrap();
            }
        }
    }
}

pub(super) fn models(
    dir: &Path,
    fixture: &Fixture,
    base: &str,
    host: &Arc<FakeHost>,
) -> Arc<LocalModels> {
    let host: Arc<dyn Host> = Arc::clone(host) as Arc<dyn Host>;
    LocalModels::new(
        dir.to_path_buf(),
        fixture.sources,
        AnalysisSupport::Available,
        base.to_string(),
        host,
    )
}

pub(super) async fn settled(models: &LocalModels) -> LocalModelState {
    models.settled().await
}
