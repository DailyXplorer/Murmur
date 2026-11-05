//! Reads the AAC file `audio.m4a` back, with `symphonia` (AAC in an MP4
//! container, both already compiled in for audio playback): its length, a
//! full decode to prove the encoder's output holds as much audio as the
//! tracks, and the tracks restored as 16 kHz mono WAVs.

use anyhow::{anyhow, bail, Result};
use std::fs::File;
use std::io::{BufWriter, ErrorKind};
use std::path::{Path, PathBuf};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::TimeBase;

use super::resample::{self, Resampler, ENCODED_RATE, TRACK_RATE};

/// The decoded audio of one packet: interleaved samples.
struct Block<'a> {
    samples: &'a [f32],
    channels: usize,
    rate: u32,
}

fn open(path: &Path) -> Result<Box<dyn FormatReader>> {
    let file = File::open(path).map_err(|_| anyhow!("Cannot open the meeting audio"))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| anyhow!("Cannot open the meeting audio"))?;
    Ok(probed.format)
}

fn milliseconds(base: TimeBase, timestamp: u64) -> u64 {
    let time = base.calc_time(timestamp);
    time.seconds * 1000 + (time.frac * 1000.0) as u64
}

/// The length of the file in milliseconds, from the container's header, or
/// from its packets when the header holds none.
pub fn duration_ms(path: &Path) -> Result<u64> {
    let mut format = open(path)?;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("The meeting audio has no track"))?;
    let params = &track.codec_params;
    let base = params
        .time_base
        .or_else(|| params.sample_rate.map(|rate| TimeBase::new(1, rate)))
        .ok_or_else(|| anyhow!("Cannot read the meeting audio length"))?;
    if let Some(frames) = params.n_frames.filter(|frames| *frames > 0) {
        return Ok(milliseconds(base, frames));
    }
    let id = track.id;
    let mut total = 0;
    loop {
        match format.next_packet() {
            Ok(packet) if packet.track_id() == id => total += packet.dur(),
            Ok(_) => {}
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break;
            }
            Err(_) => bail!("Cannot read the meeting audio length"),
        }
    }
    Ok(milliseconds(base, total))
}

/// Decodes the whole file and hands each block to `sink`.
fn read_blocks(path: &Path, mut sink: impl FnMut(Block<'_>) -> Result<()>) -> Result<()> {
    let mut format = open(path)?;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("The meeting audio has no track"))?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| anyhow!("Cannot decode the meeting audio"))?;
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                return Ok(());
            }
            Err(_) => bail!("Cannot read the meeting audio"),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|_| anyhow!("Cannot decode the meeting audio"))?;
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        let needed = decoded.capacity() * channels;
        if buffer.as_ref().is_none_or(|held| held.capacity() < needed) {
            buffer = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
        }
        let Some(held) = buffer.as_mut() else {
            bail!("Cannot decode the meeting audio");
        };
        held.copy_interleaved_ref(decoded);
        sink(Block {
            samples: held.samples(),
            channels,
            rate: spec.rate,
        })?;
    }
}

/// Decodes the whole file and returns how much audio it holds, in
/// milliseconds. An error means the file does not decode.
pub fn decoded_ms(path: &Path) -> Result<u64> {
    let (mut frames, mut rate) = (0u64, 0u32);
    read_blocks(path, |block| {
        frames += (block.samples.len() / block.channels.max(1)) as u64;
        rate = block.rate;
        Ok(())
    })?;
    if frames == 0 || rate == 0 {
        bail!("The meeting audio is empty");
    }
    Ok(frames * 1000 / u64::from(rate))
}

/// A restored track: filtered down to 16 kHz and written as it comes.
struct TrackOut {
    resampler: Resampler,
    writer: hound::WavWriter<BufWriter<File>>,
    scratch: Vec<f32>,
    converted: Vec<f32>,
}

impl TrackOut {
    fn create(path: &Path) -> Result<Self> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TRACK_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writer = hound::WavWriter::create(path, spec)
            .map_err(|_| anyhow!("Cannot create the meeting audio file"))?;
        Ok(Self {
            resampler: Resampler::down(),
            writer,
            scratch: Vec::new(),
            converted: Vec::new(),
        })
    }

    fn push(&mut self, block: &Block<'_>, channel: usize) -> Result<()> {
        self.scratch.clear();
        self.scratch.extend(
            block
                .samples
                .iter()
                .skip(channel)
                .step_by(block.channels)
                .copied(),
        );
        self.resampler.push(&self.scratch, &mut self.converted);
        write_samples(&mut self.writer, &mut self.converted)
    }

    fn finish(self) -> Result<()> {
        let Self {
            resampler,
            mut writer,
            mut converted,
            ..
        } = self;
        resampler.finish(&mut converted);
        write_samples(&mut writer, &mut converted)?;
        writer
            .finalize()
            .map_err(|_| anyhow!("Cannot read or write the meeting audio"))
    }
}

/// Writes `samples` as 16-bit and empties it.
fn write_samples(
    writer: &mut hound::WavWriter<BufWriter<File>>,
    samples: &mut Vec<f32>,
) -> Result<()> {
    for sample in samples.drain(..) {
        writer
            .write_sample(resample::to_i16(sample))
            .map_err(|_| anyhow!("Cannot read or write the meeting audio"))?;
    }
    Ok(())
}

/// Decodes the AAC file `input` into one 16 kHz mono WAV per channel,
/// `outputs` in channel order. The file must have as many channels as
/// paths are given, at the rate the archive writes.
pub fn decode(input: &Path, outputs: &[PathBuf]) -> Result<()> {
    if outputs.is_empty() || outputs.len() > 2 {
        bail!("A meeting has one or two tracks");
    }
    let mut tracks: Vec<TrackOut> = Vec::new();
    read_blocks(input, |block| {
        if tracks.is_empty() {
            if block.channels != outputs.len() || block.rate != ENCODED_RATE {
                bail!("The meeting audio has other tracks than expected");
            }
            for path in outputs {
                tracks.push(TrackOut::create(path)?);
            }
        }
        for (channel, track) in tracks.iter_mut().enumerate() {
            track.push(&block, channel)?;
        }
        Ok(())
    })?;
    if tracks.is_empty() {
        bail!("The meeting audio is empty");
    }
    tracks.into_iter().try_for_each(TrackOut::finish)
}
