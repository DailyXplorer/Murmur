//! Decodes an audio file into the 16 kHz mono WAV a meeting track is.
//!
//! The file streams through: a packet is decoded, mixed down to one channel,
//! resampled and written before the next one is read, so a two-hour podcast
//! never sits in memory (a few hundred kilobytes do). The container and the
//! codec are found by `symphonia` from the file's content; the extension is
//! only a hint.

use std::fs::File;
use std::io::{self, BufWriter, ErrorKind, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use rubato::{FftFixedIn, Resampler};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// The rate of a meeting's tracks.
pub const TRACK_RATE: u32 = 16_000;

/// The extensions the file dialog offers. The decoder does not depend on
/// them: it reads the content.
pub const EXTENSIONS: [&str; 9] = [
    "wav", "m4a", "mp3", "aif", "aiff", "aifc", "caf", "flac", "mp4",
];

/// Input samples per resampler call.
const CHUNK_FRAMES: usize = 1024;

/// Packets that fail to decode one after the other before the file is
/// called corrupt. A lost packet is a click, not the end of the import.
const MAX_BAD_PACKETS_IN_A_ROW: u32 = 50;

/// Why a file did not become a track.
#[derive(Debug)]
pub enum DecodeError {
    /// The file cannot be opened or read.
    Unreadable(io::Error),
    /// No container or codec here reads it.
    Unsupported,
    /// It decodes to nothing.
    Empty,
    /// It plays longer than the limit given.
    TooLong,
    /// It stopped making sense part-way.
    Corrupt,
    /// The track cannot be written.
    Write(io::Error),
    /// The caller asked to stop.
    Cancelled,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(error) => write!(f, "cannot read the file: {error}"),
            Self::Unsupported => f.write_str("the format is not supported"),
            Self::Empty => f.write_str("the file holds no audio"),
            Self::TooLong => f.write_str("the audio is too long"),
            Self::Corrupt => f.write_str("the file is damaged"),
            Self::Write(error) => write!(f, "cannot write the track: {error}"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// What a finished decode wrote.
#[derive(Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Samples of the 16 kHz track.
    pub samples: u64,
}

impl Decoded {
    pub fn duration_ms(&self) -> u64 {
        self.samples * 1000 / u64::from(TRACK_RATE)
    }
}

/// The file as `symphonia` reads it, counting how far it got so the caller
/// can tell the share done whatever the container.
struct Tracked {
    file: File,
    position: Arc<AtomicU64>,
    length: u64,
}

impl Read for Tracked {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buf)?;
        self.position.fetch_add(read as u64, Ordering::Relaxed);
        Ok(read)
    }
}

impl Seek for Tracked {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let at = self.file.seek(to)?;
        self.position.store(at, Ordering::Relaxed);
        Ok(at)
    }
}

impl MediaSource for Tracked {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}

/// Resamples one channel to 16 kHz, streaming. The filter's delay is dropped
/// from the start and the tail is flushed, so the output is as long as the
/// input says (`input_frames * 16000 / rate`) and lines up with it.
struct ToTrackRate {
    resampler: Option<FftFixedIn<f32>>,
    waiting: Vec<f32>,
    skip: usize,
    /// Samples still to deliver.
    owed: u64,
    delivered: u64,
}

impl ToTrackRate {
    fn new(rate: u32) -> Result<Self, DecodeError> {
        let resampler = (rate != TRACK_RATE)
            .then(|| FftFixedIn::<f32>::new(rate as usize, TRACK_RATE as usize, CHUNK_FRAMES, 2, 1))
            .transpose()
            .map_err(|_| DecodeError::Unsupported)?;
        let skip = resampler.as_ref().map_or(0, Resampler::output_delay);
        Ok(Self {
            resampler,
            waiting: Vec::with_capacity(CHUNK_FRAMES),
            skip,
            owed: u64::MAX,
            delivered: 0,
        })
    }

    fn emit(&mut self, mut converted: &[f32], out: &mut Vec<f32>) {
        if self.skip > 0 {
            let dropped = self.skip.min(converted.len());
            self.skip -= dropped;
            converted = &converted[dropped..];
        }
        let room = self.owed.saturating_sub(self.delivered);
        let take = converted
            .len()
            .min(usize::try_from(room).unwrap_or(usize::MAX));
        out.extend_from_slice(&converted[..take]);
        self.delivered += take as u64;
    }

    fn push(&mut self, mut samples: &[f32], out: &mut Vec<f32>) -> Result<(), DecodeError> {
        if self.resampler.is_none() {
            self.emit(samples, out);
            return Ok(());
        }
        while !samples.is_empty() {
            let wanted = self
                .resampler
                .as_ref()
                .map_or(CHUNK_FRAMES, Resampler::input_frames_next);
            let take = (wanted - self.waiting.len()).min(samples.len());
            self.waiting.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.waiting.len() == wanted {
                let resampler = self.resampler.as_mut().ok_or(DecodeError::Unsupported)?;
                let converted = resampler
                    .process(&[&self.waiting[..]], None)
                    .map_err(|_| DecodeError::Corrupt)?;
                self.waiting.clear();
                self.emit(&converted[0], out);
            }
        }
        Ok(())
    }

    /// Ends the stream after `input_frames` samples in all.
    fn finish(
        &mut self,
        input_frames: u64,
        rate: u32,
        out: &mut Vec<f32>,
    ) -> Result<(), DecodeError> {
        self.owed = (input_frames * u64::from(TRACK_RATE)).div_ceil(u64::from(rate));
        if self.resampler.is_none() {
            return Ok(());
        }
        // The tail, padded with silence, then silence until the delayed end
        // of the signal has come out.
        let mut tail = Some(std::mem::take(&mut self.waiting));
        for _ in 0..4 {
            if self.delivered >= self.owed {
                break;
            }
            let resampler = self.resampler.as_mut().ok_or(DecodeError::Unsupported)?;
            let converted = match tail.take() {
                Some(tail) if !tail.is_empty() => {
                    resampler.process_partial(Some(&[&tail[..]]), None)
                }
                _ => resampler.process_partial::<&[f32]>(None, None),
            }
            .map_err(|_| DecodeError::Corrupt)?;
            self.emit(&converted[0], out);
        }
        // Rounding in the resampler can leave a sample or two short.
        while self.delivered < self.owed {
            out.push(0.0);
            self.delivered += 1;
        }
        Ok(())
    }
}

/// An open file: the reader, the bytes consumed so far, the bytes in all.
type Opened = (Box<dyn FormatReader>, Arc<AtomicU64>, u64);

fn open(path: &Path) -> Result<Opened, DecodeError> {
    let file = File::open(path).map_err(DecodeError::Unreadable)?;
    let length = file.metadata().map_err(DecodeError::Unreadable)?.len();
    let position = Arc::new(AtomicU64::new(0));
    let source = Tracked {
        file,
        position: Arc::clone(&position),
        length,
    };
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| match error {
            SymphoniaError::IoError(error) => DecodeError::Unreadable(error),
            _ => DecodeError::Unsupported,
        })?;
    Ok((probed.format, position, length))
}

/// The CAF reader hands the AAC decoder the whole `esds` descriptor where the
/// decoder wants the `AudioSpecificConfig` inside it (an MP4 file gives it
/// that). Without this, every AAC in CAF is refused as "too complex".
fn unwrap_audio_specific_config(params: &mut symphonia::core::codecs::CodecParameters) {
    use symphonia::core::codecs::CODEC_TYPE_AAC;
    if params.codec != CODEC_TYPE_AAC {
        return;
    }
    let Some(config) = params.extra_data.as_deref().and_then(audio_specific_config) else {
        return;
    };
    params.extra_data = Some(config.into());
}

/// The `DecoderSpecificInfo` (tag 5) of an `ES_Descriptor` (tag 3), if the
/// bytes are one: tag, a length of 1-4 bytes (7 bits each, the high bit says
/// more follow), then the body.
fn audio_specific_config(esds: &[u8]) -> Option<Vec<u8>> {
    fn descriptor(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let (&tag, rest) = bytes.split_first()?;
        let (mut length, mut used) = (0usize, 0);
        loop {
            let byte = *rest.get(used)?;
            length = length << 7 | usize::from(byte & 0x7f);
            used += 1;
            if byte & 0x80 == 0 {
                break;
            }
            if used == 4 {
                return None;
            }
        }
        let body = rest.get(used..used + length)?;
        Some((tag, body, &rest[used + length..]))
    }
    let (tag, es, _) = descriptor(esds)?;
    if tag != 0x03 {
        return None;
    }
    // ES_ID (2), flags (1), then the optional parts the flags announce.
    let flags = *es.get(2)?;
    let mut at = 3;
    if flags & 0x80 != 0 {
        at += 2;
    }
    if flags & 0x40 != 0 {
        at += usize::from(*es.get(at)?) + 1;
    }
    if flags & 0x20 != 0 {
        at += 2;
    }
    let (tag, config, _) = descriptor(es.get(at..)?)?;
    if tag != 0x04 {
        return None;
    }
    // objectType (1), stream type + buffer size (4), max and average rate (8).
    let (tag, asc, _) = descriptor(config.get(13..)?)?;
    (tag == 0x05 && !asc.is_empty()).then(|| asc.to_vec())
}

/// Writes the audio of `source` to `target` as a 16 kHz, 16-bit mono WAV.
///
/// `progress` is called with the share of the file read, in `0.0..=1.0`, now
/// and then. `cancel` set stops the decode at the next packet; `target` is
/// left as it was (the caller deletes it). Audio longer than `max_ms` is
/// refused as soon as that is known.
pub fn decode_to_wav(
    source: &Path,
    target: &Path,
    max_ms: u64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<Decoded, DecodeError> {
    let (mut format, position, length) = open(source)?;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or(DecodeError::Unsupported)?;
    let id = track.id;
    let mut params = track.codec_params.clone();
    unwrap_audio_specific_config(&mut params);
    if let (Some(frames), Some(rate)) = (params.n_frames, params.sample_rate) {
        if rate > 0 && frames * 1000 / u64::from(rate) > max_ms {
            return Err(DecodeError::TooLong);
        }
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|_| DecodeError::Unsupported)?;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TRACK_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let file = File::create(target).map_err(DecodeError::Write)?;
    let mut writer =
        hound::WavWriter::new(BufWriter::new(file), spec).map_err(|error| write_error(&error))?;

    let mut buffer: Option<SampleBuffer<f32>> = None;
    let mut converter: Option<(ToTrackRate, u32)> = None;
    let (mut input_frames, mut written) = (0u64, 0u64);
    let (mut mono, mut converted) = (Vec::<f32>::new(), Vec::<f32>::new());
    let mut bad_in_a_row = 0;
    let mut reported = -1.0f32;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DecodeError::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break
            }
            // A chained stream (Ogg) or a track list that changed: what was
            // read is what there is.
            Err(SymphoniaError::ResetRequired) => break,
            Err(SymphoniaError::IoError(error)) => return Err(DecodeError::Unreadable(error)),
            Err(_) => return Err(DecodeError::Corrupt),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => {
                bad_in_a_row = 0;
                decoded
            }
            Err(SymphoniaError::DecodeError(_)) => {
                bad_in_a_row += 1;
                if bad_in_a_row > MAX_BAD_PACKETS_IN_A_ROW {
                    return Err(DecodeError::Corrupt);
                }
                continue;
            }
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break
            }
            Err(SymphoniaError::Unsupported(_)) => return Err(DecodeError::Unsupported),
            Err(_) => return Err(DecodeError::Corrupt),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        if channels == 0 || spec.rate == 0 {
            return Err(DecodeError::Unsupported);
        }
        let needed = decoded.capacity();
        if buffer
            .as_ref()
            .is_none_or(|held| held.capacity() < needed * channels)
        {
            buffer = Some(SampleBuffer::new(needed as u64, spec));
        }
        let held = buffer.as_mut().ok_or(DecodeError::Corrupt)?;
        held.copy_interleaved_ref(decoded);
        mono.clear();
        mono.extend(
            held.samples()
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        if converter
            .as_ref()
            .is_some_and(|(_, rate)| *rate != spec.rate)
        {
            // The rate changed part-way: not a file a meeting can be made of.
            return Err(DecodeError::Unsupported);
        }
        if converter.is_none() {
            converter = Some((ToTrackRate::new(spec.rate)?, spec.rate));
        }
        let (resampler, rate) = converter.as_mut().ok_or(DecodeError::Corrupt)?;
        input_frames += mono.len() as u64;
        if input_frames * 1000 / u64::from(*rate) > max_ms {
            return Err(DecodeError::TooLong);
        }
        converted.clear();
        resampler.push(&mono, &mut converted)?;
        written += write_samples(&mut writer, &converted)?;

        let share = if length == 0 {
            0.0
        } else {
            (position.load(Ordering::Relaxed) as f32 / length as f32).clamp(0.0, 1.0)
        };
        if share - reported >= 0.005 {
            reported = share;
            progress(share);
        }
    }
    let Some((mut resampler, rate)) = converter else {
        return Err(DecodeError::Empty);
    };
    converted.clear();
    resampler.finish(input_frames, rate, &mut converted)?;
    written += write_samples(&mut writer, &converted)?;
    if written == 0 {
        return Err(DecodeError::Empty);
    }
    // Writes the lengths into the header, and flushes.
    writer.finalize().map_err(|error| write_error(&error))?;
    progress(1.0);
    Ok(Decoded { samples: written })
}

fn write_error(error: &hound::Error) -> DecodeError {
    match error {
        hound::Error::IoError(error) => {
            DecodeError::Write(io::Error::new(error.kind(), error.to_string()))
        }
        other => DecodeError::Write(io::Error::other(other.to_string())),
    }
}

fn write_samples<W: io::Write + Seek>(
    writer: &mut hound::WavWriter<W>,
    samples: &[f32],
) -> Result<u64, DecodeError> {
    for sample in samples {
        let value = (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
        writer
            .write_sample(value)
            .map_err(|error| write_error(&error))?;
    }
    Ok(samples.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;
    use std::io::Write;

    const TONE_HZ: f64 = 440.0;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("src/meetings/import_audio/fixtures/{name}"))
    }

    fn samples_of(path: &Path) -> (hound::WavSpec, Vec<f32>) {
        let mut reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        let samples = reader
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / 32_768.0)
            .collect();
        (spec, samples)
    }

    /// The share of the signal's power within a hertz or two of `hz`: how
    /// much of what was decoded is the tone (a cut-off or odd-length signal
    /// leaks a little to the sides, hence the search).
    fn share_at(samples: &[f32], hz: f64) -> f64 {
        (-8..=8)
            .map(|step| share_exactly_at(samples, hz + f64::from(step) * 0.25))
            .fold(0.0, f64::max)
    }

    fn share_exactly_at(samples: &[f32], hz: f64) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (at, sample) in samples.iter().enumerate() {
            let phase = 2.0 * PI * hz * at as f64 / f64::from(TRACK_RATE);
            re += f64::from(*sample) * phase.cos();
            im += f64::from(*sample) * phase.sin();
        }
        let n = samples.len() as f64;
        let power_at = 2.0 * (re * re + im * im) / (n * n);
        let total = samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / n;
        power_at / total
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
    }

    fn decode(source: &Path) -> Result<(Decoded, Vec<f32>), DecodeError> {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("system.wav");
        let decoded = decode_to_wav(source, &target, u64::MAX, &AtomicBool::new(false), |_| {})?;
        let (spec, samples) = samples_of(&target);
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, TRACK_RATE, 16)
        );
        assert_eq!(samples.len() as u64, decoded.samples);
        Ok((decoded, samples))
    }

    /// Writes a WAV of a tone at `rate`, `seconds` long.
    fn write_wav(
        path: &Path,
        rate: u32,
        channels: u16,
        format: (u16, hound::SampleFormat),
        seconds: f64,
        amplitude: f64,
    ) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: format.0,
            sample_format: format.1,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for at in 0..(f64::from(rate) * seconds).round() as u64 {
            let value = amplitude * (2.0 * PI * TONE_HZ * at as f64 / f64::from(rate)).sin();
            for _ in 0..channels {
                match format {
                    (8, _) => writer.write_sample((value * 127.0) as i8).unwrap(),
                    (16, _) => writer.write_sample((value * 32_767.0) as i16).unwrap(),
                    (24, _) => writer.write_sample((value * 8_388_607.0) as i32).unwrap(),
                    (32, hound::SampleFormat::Float) => writer.write_sample(value as f32).unwrap(),
                    _ => unreachable!(),
                }
            }
        }
        writer.finalize().unwrap();
    }

    const INT16: (u16, hound::SampleFormat) = (16, hound::SampleFormat::Int);

    #[test]
    fn every_supported_container_decodes_to_the_tone_at_16_khz() {
        for name in [
            "tone-44k-stereo.mp3",
            "tone-24k-mono.flac",
            "tone-44k-stereo-aac.m4a",
            "tone-22k-mono-alac.m4a",
            "tone-8k-mono-alac.caf",
            "tone-24k-mono-aac.caf",
            "tone-8k-mono-pcm.caf",
            "tone-8k-mono-pcm.aiff",
        ] {
            let (decoded, samples) = decode(&fixture(name)).unwrap_or_else(|error| {
                panic!("{name}: {error}");
            });
            // Lossy codecs pad their ends by a frame or two (AAC in CAF keeps its
            // priming frames: the format has no field `symphonia` reads for them).
            let ms = decoded.duration_ms();
            assert!((380..=560).contains(&ms), "{name}: {ms} ms");
            let level = rms(&samples);
            assert!((0.04..=0.12).contains(&level), "{name}: rms {level}");
            assert!(
                share_at(&samples, TONE_HZ) > 0.7,
                "{name}: the tone is lost ({})",
                share_at(&samples, TONE_HZ)
            );
        }
    }

    #[test]
    fn a_wav_of_any_depth_rate_and_channels_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (8_000, 1, (8, hound::SampleFormat::Int)),
            (16_000, 1, INT16),
            (44_100, 2, INT16),
            (48_000, 1, (24, hound::SampleFormat::Int)),
            (22_050, 2, (32, hound::SampleFormat::Float)),
            (96_000, 1, INT16),
        ];
        for (rate, channels, format) in cases {
            let source = dir.path().join("in.wav");
            write_wav(&source, rate, channels, format, 1.5, 0.5);
            let (decoded, samples) = decode(&source).unwrap_or_else(|error| {
                panic!("{rate} Hz, {channels} ch, {format:?}: {error}");
            });
            // Exactly the length the input says, whatever the filter delays.
            let frames = (f64::from(rate) * 1.5).round() as u64;
            assert_eq!(
                decoded.samples,
                (frames * u64::from(TRACK_RATE)).div_ceil(u64::from(rate)),
                "{rate} Hz"
            );
            let share = share_at(&samples, TONE_HZ);
            assert!(share > 0.9, "{rate} Hz, {format:?}: tone share {share}");
            let level = rms(&samples);
            let want = 0.5 / 2f64.sqrt();
            assert!((level - want).abs() < 0.03, "{rate} Hz: rms {level}");
        }
    }

    #[test]
    fn a_resampled_tone_starts_at_the_start() {
        // The resampler's delay is removed: the first quarter second holds
        // the tone at full level, not a ramp or silence.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.wav");
        write_wav(&source, 44_100, 1, INT16, 1.0, 0.5);
        let (_, samples) = decode(&source).unwrap();
        let head = &samples[..TRACK_RATE as usize / 4];
        assert!(rms(head) > 0.3, "{}", rms(head));
        let tail = &samples[samples.len() - TRACK_RATE as usize / 4..];
        assert!(rms(tail) > 0.3, "{}", rms(tail));
    }

    #[test]
    fn a_long_file_streams_to_the_exact_length() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.wav");
        write_wav(&source, 44_100, 2, INT16, 600.0, 0.3);
        let (decoded, _) = decode(&source).unwrap();
        assert_eq!(decoded.samples, 600 * u64::from(TRACK_RATE));
    }

    #[test]
    fn progress_climbs_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.wav");
        write_wav(&source, 16_000, 1, INT16, 30.0, 0.3);
        let mut seen = Vec::new();
        decode_to_wav(
            &source,
            &dir.path().join("out.wav"),
            u64::MAX,
            &AtomicBool::new(false),
            |share| seen.push(share),
        )
        .unwrap();
        assert!(seen.len() > 3, "{seen:?}");
        assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&1.0));
    }

    #[test]
    fn a_cancel_stops_the_decode() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.wav");
        write_wav(&source, 16_000, 1, INT16, 30.0, 0.3);
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let result = decode_to_wav(
            &source,
            &dir.path().join("out.wav"),
            u64::MAX,
            &cancel,
            |_| {
                calls += 1;
                if calls == 2 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(matches!(result, Err(DecodeError::Cancelled)));
        assert!(calls < 20, "{calls}");
    }

    #[test]
    fn audio_over_the_limit_is_refused_with_its_length_known() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.wav");
        write_wav(&source, 16_000, 1, INT16, 3.0, 0.3);
        let result = decode_to_wav(
            &source,
            &dir.path().join("out.wav"),
            2_000,
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(matches!(result, Err(DecodeError::TooLong)), "{result:?}");
    }

    #[test]
    fn audio_over_the_limit_is_refused_when_its_length_is_not_in_the_header() {
        // An MP3 has no length: the limit is found while decoding.
        let result = decode_to_wav(
            &fixture("tone-44k-stereo.mp3"),
            &tempfile::tempdir().unwrap().path().join("out.wav"),
            100,
            &AtomicBool::new(false),
            |_| {},
        );
        assert!(matches!(result, Err(DecodeError::TooLong)), "{result:?}");
    }

    #[test]
    fn what_is_not_audio_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("notes.mp3");
        std::fs::write(&source, "Meeting notes, not sound.\n".repeat(400)).unwrap();
        assert!(matches!(decode(&source), Err(DecodeError::Unsupported)));
        let empty = dir.path().join("empty.wav");
        std::fs::File::create(&empty).unwrap();
        assert!(matches!(
            decode(&empty),
            Err(DecodeError::Unsupported | DecodeError::Empty)
        ));
    }

    #[test]
    fn a_missing_file_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            decode(&dir.path().join("gone.wav")),
            Err(DecodeError::Unreadable(_))
        ));
    }

    #[test]
    fn a_wav_with_no_samples_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.wav");
        write_wav(&source, 16_000, 1, INT16, 0.0, 0.3);
        assert!(matches!(decode(&source), Err(DecodeError::Empty)));
    }

    #[test]
    fn a_cut_off_file_keeps_what_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let whole = dir.path().join("whole.wav");
        write_wav(&whole, 16_000, 1, INT16, 4.0, 0.3);
        let mut bytes = std::fs::read(&whole).unwrap();
        bytes.truncate(bytes.len() / 2);
        let cut = dir.path().join("cut.wav");
        std::fs::File::create(&cut)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        let (decoded, samples) = decode(&cut).unwrap();
        assert!(
            (1_800..=2_100).contains(&decoded.duration_ms()),
            "{decoded:?}"
        );
        assert!(share_at(&samples, TONE_HZ) > 0.95);

        let mp3 = std::fs::read(fixture("tone-44k-stereo.mp3")).unwrap();
        let cut = dir.path().join("cut.mp3");
        std::fs::write(&cut, &mp3[..mp3.len() * 2 / 3]).unwrap();
        let (decoded, _) = decode(&cut).unwrap();
        assert!(decoded.duration_ms() > 150, "{decoded:?}");
    }

    #[test]
    fn the_extension_is_only_a_hint() {
        let dir = tempfile::tempdir().unwrap();
        for (name, as_name) in [
            ("tone-24k-mono.flac", "recording.wav"),
            ("tone-44k-stereo.mp3", "recording.m4a"),
            ("tone-44k-stereo-aac.m4a", "recording"),
        ] {
            let copy = dir.path().join(as_name);
            std::fs::copy(fixture(name), &copy).unwrap();
            let (decoded, _) = decode(&copy).unwrap_or_else(|error| panic!("{as_name}: {error}"));
            assert!((380..=480).contains(&decoded.duration_ms()), "{as_name}");
        }
    }
}
