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
    pub files: &'static [ModelFile],
}

impl Source {
    pub fn url(&self, base: &str, file: &ModelFile) -> String {
        format!("{base}/{}/resolve/{}/{}", self.repo, self.commit, file.path)
    }
}

/// Apple silicon runs the Core ML models through FluidAudio, the other Macs
/// the GGUF file on the CPU.
pub fn this_mac() -> &'static [Source] {
    if cfg!(target_arch = "aarch64") {
        pinned::APPLE_SILICON
    } else {
        pinned::X86_64
    }
}

pub fn total_bytes(sources: &[Source]) -> u64 {
    sources
        .iter()
        .flat_map(|source| source.files)
        .map(|file| file.size)
        .sum()
}

/// Held by the ready marker, so a new pin has the folder checked again.
pub fn id(sources: &[Source]) -> String {
    sources
        .iter()
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
        for source in pinned::APPLE_SILICON.iter().chain(pinned::X86_64) {
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
        assert_eq!(total_bytes(pinned::APPLE_SILICON), 505_044_208);
        assert_eq!(total_bytes(pinned::X86_64), 739_508_576);
    }

    #[test]
    fn fluid_audio_finds_the_core_ml_models_where_it_looks() {
        let folders: Vec<_> = pinned::APPLE_SILICON.iter().map(|s| s.folder).collect();
        assert_eq!(folders, ["parakeet-tdt-0.6b-v3", "speaker-diarization"]);
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
            "https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf/resolve/\
             d8d8c32cb2017c57e5307767343e97870f8a7f53/parakeet-tdt-0.6b-v3-Q8_0.gguf"
        );
    }
}
