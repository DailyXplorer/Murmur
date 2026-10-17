//! Reads a meeting track from disk a block at a time: a three-hour track is
//! about 350 MB of samples and never sits in memory whole.

use anyhow::{anyhow, Context, Result};
use std::path::Path;

use crate::audio_toolkit::constants::TARGET_SAMPLE_RATE;
use crate::audio_toolkit::gain::{frame_levels, FRAME};

const SAMPLES_PER_MS: u64 = TARGET_SAMPLE_RATE as u64 / 1000;
/// Ten seconds of samples, a whole number of 20 ms frames.
const BLOCK: usize = FRAME * 500;

pub struct TrackLevels {
    pub duration_ms: u64,
    /// One level in dBFS per 20 ms.
    pub levels: Vec<f32>,
}

fn open(path: &Path) -> Result<hound::WavReader<std::io::BufReader<std::fs::File>>> {
    let reader =
        hound::WavReader::open(path).with_context(|| format!("Cannot open {}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != TARGET_SAMPLE_RATE
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(anyhow!("{} is not a 16 kHz mono track", path.display()));
    }
    Ok(reader)
}

fn to_f32(sample: i16) -> f32 {
    f32::from(sample) / f32::from(i16::MAX)
}

pub fn levels(path: &Path) -> Result<TrackLevels> {
    let mut reader = open(path)?;
    let total = u64::from(reader.duration());
    let mut levels = Vec::with_capacity((total / FRAME as u64 + 1) as usize);
    let mut block = Vec::with_capacity(BLOCK);
    for sample in reader.samples::<i16>() {
        block.push(to_f32(sample?));
        if block.len() == BLOCK {
            levels.extend(frame_levels(&block));
            block.clear();
        }
    }
    levels.extend(frame_levels(&block));
    Ok(TrackLevels {
        duration_ms: total / SAMPLES_PER_MS,
        levels,
    })
}

/// The samples of `[start_ms, end_ms)`, cut short at the end of the track.
pub fn read_range(path: &Path, start_ms: u64, end_ms: u64) -> Result<Vec<f32>> {
    let mut reader = open(path)?;
    let total = u64::from(reader.duration());
    let start = (start_ms * SAMPLES_PER_MS).min(total);
    let end = (end_ms * SAMPLES_PER_MS).clamp(start, total);
    reader.seek(start as u32)?;
    reader
        .samples::<i16>()
        .take((end - start) as usize)
        .map(|sample| Ok(to_f32(sample?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TARGET_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn levels_cover_the_track_in_20_ms_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.wav");
        // 12.5 s: one second of loud audio, then silence.
        let mut samples = vec![0i16; 200_000];
        for (index, sample) in samples[..16_000].iter_mut().enumerate() {
            *sample = if index % 2 == 0 { 16_000 } else { -16_000 };
        }
        write(&path, &samples);

        let track = levels(&path).unwrap();
        assert_eq!(track.duration_ms, 12_500);
        assert_eq!(track.levels.len(), 625);
        assert!(track.levels[..50].iter().all(|level| *level > -10.0));
        assert!(track.levels[50..].iter().all(|level| *level < -100.0));
    }

    #[test]
    fn a_range_is_read_from_its_position() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        let samples: Vec<i16> = (0..48_000).map(|index| (index / 16) as i16).collect();
        write(&path, &samples);

        let range = read_range(&path, 1_000, 1_002).unwrap();
        assert_eq!(range.len(), 32);
        assert_eq!(range[0], to_f32(1_000));
        assert_eq!(range[31], to_f32(1_001));
        assert_eq!(read_range(&path, 2_900, 9_000).unwrap().len(), 1_600);
        assert!(read_range(&path, 5_000, 6_000).unwrap().is_empty());
        assert!(read_range(&dir.path().join("missing.wav"), 0, 1).is_err());
    }
}
