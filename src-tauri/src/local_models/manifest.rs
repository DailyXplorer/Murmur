//! What the local models are: every file at one pinned Hugging Face commit,
//! with its size and SHA-256. The lists live in `pinned.rs`, written by
//! `scripts/generate-model-manifest.ts`.

use super::pinned;

pub const HUGGING_FACE: &str = "https://huggingface.co";

#[derive(Debug)]
pub struct ModelFile {
    /// Relative to the source's folder, as in the repository.
    pub path: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

#[derive(Debug)]
pub struct Source {
    pub repo: &'static str,
    pub commit: &'static str,
    /// The folder under the models directory, named as FluidAudio expects.
    pub folder: &'static str,
    /// FluidAudio loads this repository only with a `.fluidaudio-revision`
    /// marker naming `commit`.
    pub fluid_revision_marker: bool,
    /// Not needed to dictate: fetched once the others are ready, and never
    /// keeps them from being ready.
    pub optional: bool,
    pub files: &'static [ModelFile],
}

impl Source {
    pub fn url(&self, base: &str, file: &ModelFile) -> String {
        format!("{base}/{}/resolve/{}/{}", self.repo, self.commit, file.path)
    }
}

/// Apple silicon runs the Core ML models through FluidAudio, the other Macs
/// the GGUF file on the CPU. Windows runs the same GGUF file, and fetches the
/// speaker model beside it (optional: dictation never waits for it).
pub fn this_mac() -> &'static [Source] {
    if cfg!(target_os = "windows") {
        pinned::WINDOWS
    } else if cfg!(target_arch = "aarch64") {
        pinned::APPLE_SILICON
    } else {
        pinned::X86_64
    }
}

/// The speaker model (Sortformer) relative to the models directory, on
/// Windows, where it is the one optional source.
#[cfg(target_os = "windows")]
pub fn speaker_gguf_path() -> std::path::PathBuf {
    let source = &pinned::WINDOWS[1];
    std::path::Path::new(source.folder).join(source.files[0].path)
}

/// The speech model earlier versions downloaded, Parakeet TDT 0.6B v3. A
/// folder it left ready stays in use while Parakeet Ultra downloads beside
/// it, and is deleted once Ultra is ready.
#[derive(Debug)]
pub struct Previous {
    /// What `.murmur-ready` held once it was ready.
    pub ready_id: &'static str,
    pub folder: &'static str,
}

pub const PREVIOUS: Previous = if cfg!(target_arch = "aarch64") {
    Previous {
        ready_id: "FluidInference/parakeet-tdt-0.6b-v3-coreml@7dd20fe6b1797d35f5e3307e8b1732d9a178edfe\n\
                   FluidInference/speaker-diarization-coreml@df2625ac79a7ac6b65ad868fee6d80f320da4232",
        folder: "parakeet-tdt-0.6b-v3",
    }
} else {
    Previous {
        ready_id:
            "handy-computer/parakeet-tdt-0.6b-v3-gguf@d8d8c32cb2017c57e5307767343e97870f8a7f53",
        folder: "parakeet-tdt-0.6b-v3",
    }
};

/// Which speech model a ready models folder runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asr {
    Current,
    Previous,
}

impl Asr {
    /// The helper's `--asr` value.
    pub fn helper_name(self) -> &'static str {
        match self {
            Asr::Current => "ultra",
            Asr::Previous => "v3",
        }
    }

    /// The GGUF model the CPU loads, relative to the models directory.
    #[cfg(not(target_arch = "aarch64"))]
    pub fn gguf_path(self) -> std::path::PathBuf {
        match self {
            Asr::Current => {
                let source = &pinned::X86_64[0];
                std::path::Path::new(source.folder).join(source.files[0].path)
            }
            Asr::Previous => {
                std::path::Path::new(PREVIOUS.folder).join("parakeet-tdt-0.6b-v3-Q8_0.gguf")
            }
        }
    }
}

pub fn total_bytes<'a>(sources: impl IntoIterator<Item = &'a Source>) -> u64 {
    sources
        .into_iter()
        .flat_map(|source| source.files)
        .map(|file| file.size)
        .sum()
}

/// Held by the ready marker, so a new pin has the folder checked again.
pub fn id<'a>(sources: impl IntoIterator<Item = &'a Source>) -> String {
    sources
        .into_iter()
        .map(|source| format!("{}@{}", source.repo, source.commit))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_hex(text: &str, length: usize) -> bool {
        text.len() == length
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    #[test]
    fn every_source_is_pinned_to_a_commit_and_every_file_to_a_sha256() {
        for source in pinned::APPLE_SILICON
            .iter()
            .chain(pinned::X86_64)
            .chain(pinned::WINDOWS)
        {
            assert!(is_hex(source.commit, 40), "{} is not pinned", source.repo);
            assert!(!source.files.is_empty(), "{} has no files", source.repo);
            for file in source.files {
                assert!(is_hex(file.sha256, 64), "{}", file.path);
                assert!(file.size > 0, "{}", file.path);
                assert!(!file.path.starts_with('/') && !file.path.contains(".."));
            }
        }
    }

    #[test]
    fn the_download_sizes_are_the_ones_shown() {
        assert_eq!(total_bytes(pinned::APPLE_SILICON), 756_903_921);
        assert_eq!(total_bytes(pinned::X86_64), 740_363_168);
        assert_eq!(total_bytes(pinned::WINDOWS), 879_673_504);
    }

    /// Windows dictates with the Intel Mac model, so a models folder ready
    /// before the speaker model came stays ready: same required sources.
    #[test]
    fn windows_adds_one_optional_speaker_model_to_the_intel_set() {
        let required: Vec<_> = pinned::WINDOWS.iter().filter(|s| !s.optional).collect();
        assert_eq!(required.len(), pinned::X86_64.len());
        for (windows, intel) in required.iter().zip(pinned::X86_64) {
            assert_eq!(
                (windows.repo, windows.commit, windows.folder),
                (intel.repo, intel.commit, intel.folder)
            );
            assert_eq!(windows.files.len(), intel.files.len());
            for (a, b) in windows.files.iter().zip(intel.files) {
                assert_eq!((a.path, a.size, a.sha256), (b.path, b.size, b.sha256));
            }
        }
        assert_eq!(
            id(pinned::WINDOWS.iter().filter(|s| !s.optional)),
            id(pinned::X86_64)
        );
        let optional: Vec<_> = pinned::WINDOWS.iter().filter(|s| s.optional).collect();
        assert_eq!(optional.len(), 1);
        assert_eq!(optional[0].folder, "sortformer");
        assert_eq!(optional[0].files.len(), 1);
        assert!(optional[0].files[0].path.ends_with("Q8_0.gguf"));
    }

    /// The Meetings page shows the size the download really is, in every
    /// language, rather than a number of its own.
    #[test]
    fn the_meetings_page_quotes_the_download_size_it_is_given() {
        let locales = [
            include_str!("../../../src/i18n/locales/en/translation.json"),
            include_str!("../../../src/i18n/locales/fr/translation.json"),
            include_str!("../../../src/i18n/locales/de/translation.json"),
            include_str!("../../../src/i18n/locales/es/translation.json"),
            include_str!("../../../src/i18n/locales/it/translation.json"),
            include_str!("../../../src/i18n/locales/pt/translation.json"),
        ];
        for locale in locales {
            let json: serde_json::Value = serde_json::from_str(locale).unwrap();
            let text = json["meetings"]["models"]["description"].as_str().unwrap();
            assert!(text.contains("{{size}}"), "{text}");
            assert!(!text.contains(|c: char| c.is_ascii_digit()), "{text}");
        }
    }

    #[test]
    fn only_the_boosting_model_is_optional() {
        let optional: Vec<_> = pinned::APPLE_SILICON
            .iter()
            .chain(pinned::X86_64)
            .filter(|source| source.optional)
            .map(|source| source.folder)
            .collect();
        assert_eq!(optional, ["parakeet-ctc-110m-coreml"]);
        let ctc = &pinned::APPLE_SILICON[2];
        // FluidAudio checks for vocab.json last; it is also fetched last.
        assert_eq!(ctc.files.last().map(|file| file.path), Some("vocab.json"));
        assert!(ctc.files.iter().any(|f| f.path == "tokenizer.json"));
    }

    #[test]
    fn fluid_audio_finds_the_core_ml_models_where_it_looks() {
        let folders: Vec<_> = pinned::APPLE_SILICON.iter().map(|s| s.folder).collect();
        assert_eq!(
            folders,
            [
                "parakeet-ultra",
                "speaker-diarization",
                "parakeet-ctc-110m-coreml"
            ]
        );
        let asr = &pinned::APPLE_SILICON[0];
        for bundle in [
            "Preprocessor.mlmodelc",
            "Encoder.mlmodelc",
            "Decoder.mlmodelc",
            "JointDecisionv3.mlmodelc",
        ] {
            let coreml = format!("{bundle}/coremldata.bin");
            assert!(asr.files.iter().any(|file| file.path == coreml), "{bundle}");
        }
        assert!(asr.files.iter().any(|f| f.path == "parakeet_vocab.json"));
        let diarizer = &pinned::APPLE_SILICON[1];
        assert!(diarizer.fluid_revision_marker && !asr.fluid_revision_marker);
        assert!(diarizer
            .files
            .iter()
            .any(|f| f.path == "plda-parameters.json"));
    }

    #[test]
    fn urls_resolve_the_pinned_commit() {
        let source = &pinned::X86_64[0];
        assert_eq!(
            source.url(HUGGING_FACE, &source.files[0]),
            "https://huggingface.co/handy-computer/parakeet-ultra-gguf/resolve/\
             a1539be1b849ea7463b88343e5277b373d0c4ea9/parakeet-ultra-Q8_0.gguf"
        );
    }

    /// The previous model's folder is deleted once the current one is ready:
    /// it must hold nothing the current models need.
    #[test]
    fn the_previous_speech_model_shares_no_folder_with_the_current_models() {
        for source in pinned::APPLE_SILICON
            .iter()
            .chain(pinned::X86_64)
            .chain(pinned::WINDOWS)
        {
            assert_ne!(source.folder, PREVIOUS.folder, "{}", source.repo);
        }
        assert_ne!(
            PREVIOUS.ready_id,
            id(this_mac().iter().filter(|s| !s.optional))
        );
    }
}
