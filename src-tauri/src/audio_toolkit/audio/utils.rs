use anyhow::{bail, Result};
use hound::{WavReader, WavSpec, WavWriter};
use log::debug;
use std::path::Path;

/// Read a WAV file and return normalised f32 samples.
pub fn read_wav_samples<P: AsRef<Path>>(file_path: P) -> Result<Vec<f32>> {
    let reader = WavReader::open(file_path.as_ref())?;
    let sample_count = reader.len() as usize;
    ensure_wav_sample_count_within_limit(sample_count)?;
    let samples = reader
        .into_samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<Result<Vec<f32>, _>>()?;
    Ok(samples)
}

fn ensure_wav_sample_count_within_limit(sample_count: usize) -> Result<()> {
    if sample_count > crate::audio_toolkit::constants::MAX_RECORDING_SAMPLES {
        bail!(
            "WAV contains {sample_count} samples; the maximum is {}",
            crate::audio_toolkit::constants::MAX_RECORDING_SAMPLES
        );
    }
    Ok(())
}

/// Verify a WAV file by reading it back and checking the sample count.
pub fn verify_wav_file<P: AsRef<Path>>(file_path: P, expected_samples: usize) -> Result<()> {
    let reader = WavReader::open(file_path.as_ref())?;
    let actual_samples = reader.len() as usize;
    if actual_samples != expected_samples {
        anyhow::bail!(
            "WAV sample count mismatch: expected {}, got {}",
            expected_samples,
            actual_samples
        );
    }
    Ok(())
}

/// Save audio samples as a WAV file
pub fn save_wav_file<P: AsRef<Path>>(file_path: P, samples: &[f32]) -> Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = WavWriter::create(file_path.as_ref(), spec)?;

    // Convert f32 samples to i16 for WAV
    for sample in samples {
        let sample_i16 = (sample * i16::MAX as f32) as i16;
        writer.write_sample(sample_i16)?;
    }

    writer.finalize()?;
    debug!("Saved WAV file: {:?}", file_path.as_ref());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_wav_sample_limit_before_collecting() {
        assert!(ensure_wav_sample_count_within_limit(
            crate::audio_toolkit::constants::MAX_RECORDING_SAMPLES
        )
        .is_ok());
        assert!(ensure_wav_sample_count_within_limit(
            crate::audio_toolkit::constants::MAX_RECORDING_SAMPLES + 1
        )
        .is_err());
    }
}
