//! Writes `audio.m4a` on Windows with Media Foundation's AAC encoder and
//! MPEG-4 file sink, through the sink writer. The encoder takes 16-bit PCM at
//! 44.1 or 48 kHz only, so each 16 kHz track is brought up to 48 kHz first
//! (`resample.rs`). Its lowest bit rate is 96 kbps (12,000 bytes a second),
//! about 43 MB an hour for two channels.
//!
//! Media Foundation is loaded at run time, never linked: N and KN editions of
//! Windows, and Server without the Media Foundation feature, have no
//! `mfplat.dll`, and a link-time import would stop the whole app from
//! starting there. Without the DLLs or the encoder `encode` fails, and the
//! WAV tracks stay (see `archive`).

use anyhow::{anyhow, bail, Result};
use std::ffi::c_void;
use std::fs::File;
use std::io::BufReader;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use windows::core::{s, w, Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter, MFAudioFormat_AAC,
    MFAudioFormat_PCM, MFMediaType_Audio, MFTranscodeContainerType_MPEG4, MFSTARTUP_FULL,
    MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, MF_MT_AAC_PAYLOAD_TYPE,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_TRANSCODE_CONTAINERTYPE, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

use super::resample::{self, Resampler, ENCODED_RATE, TRACK_RATE};
use super::{aac, DURATION_TOLERANCE_MS};

/// Samples read from a track at a time.
const BLOCK: usize = 4_096;
/// The encoder's lowest bit rate, in bytes a second, for one or two channels.
const AAC_BYTES_PER_SECOND: u32 = 12_000;
/// "AAC Profile L2", the encoder's default.
const AAC_PROFILE_LEVEL: u32 = 0x29;
const HNS_PER_SECOND: u64 = 10_000_000;

type StartupFn = unsafe extern "system" fn(u32, u32) -> HRESULT;
type ShutdownFn = unsafe extern "system" fn() -> HRESULT;
type CreateAttributesFn = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type CreateObjectFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateMemoryBufferFn = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;
type CreateSinkWriterFn =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;

/// The few Media Foundation functions the encoder calls. The interfaces they
/// return are called through their tables and import nothing.
struct Api {
    startup: StartupFn,
    shutdown: ShutdownFn,
    create_attributes: CreateAttributesFn,
    create_media_type: CreateObjectFn,
    create_memory_buffer: CreateMemoryBufferFn,
    create_sample: CreateObjectFn,
    create_sink_writer: CreateSinkWriterFn,
}

const MISSING: &str = "Media Foundation is not installed";

fn load() -> std::result::Result<Api, &'static str> {
    // SAFETY: the names are NUL-terminated literals. The libraries stay
    // loaded for the life of the process, so the addresses stay valid.
    let load = |name: PCWSTR| unsafe { LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32) };
    let platform = load(w!("mfplat.dll")).map_err(|_| MISSING)?;
    let read_write = load(w!("mfreadwrite.dll")).map_err(|_| MISSING)?;
    macro_rules! function {
        ($module:expr, $name:expr, $type:ty) => {{
            // SAFETY: the exported function has the signature of `$type`.
            let address = unsafe { GetProcAddress($module, $name) }.ok_or(MISSING)?;
            unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $type>(address) }
        }};
    }
    Ok(Api {
        startup: function!(platform, s!("MFStartup"), StartupFn),
        shutdown: function!(platform, s!("MFShutdown"), ShutdownFn),
        create_attributes: function!(platform, s!("MFCreateAttributes"), CreateAttributesFn),
        create_media_type: function!(platform, s!("MFCreateMediaType"), CreateObjectFn),
        create_memory_buffer: function!(platform, s!("MFCreateMemoryBuffer"), CreateMemoryBufferFn),
        create_sample: function!(platform, s!("MFCreateSample"), CreateObjectFn),
        create_sink_writer: function!(
            read_write,
            s!("MFCreateSinkWriterFromURL"),
            CreateSinkWriterFn
        ),
    })
}

fn api() -> Result<&'static Api> {
    static API: OnceLock<std::result::Result<Api, &'static str>> = OnceLock::new();
    API.get_or_init(load)
        .as_ref()
        .map_err(|reason| anyhow!(*reason))
}

/// Turns a failed call into an error that names the step and the code, and
/// nothing about the audio.
fn done<T>(result: windows::core::Result<T>, step: &str) -> Result<T> {
    result.map_err(|error| anyhow!("{step} failed ({:#010x})", error.code().0))
}

/// Calls a `Create…` function that hands back one interface through an out
/// pointer.
fn created<T: Interface>(call: impl FnOnce(*mut *mut c_void) -> HRESULT, step: &str) -> Result<T> {
    let mut raw: *mut c_void = std::ptr::null_mut();
    done(call(&mut raw).ok(), step)?;
    if raw.is_null() {
        bail!("{step} returned nothing");
    }
    // SAFETY: the call succeeded, so `raw` is an owned reference to a `T`.
    Ok(unsafe { T::from_raw(raw) })
}

/// COM on the calling thread, balanced on drop. A thread that already chose
/// another apartment keeps it and releases nothing.
struct Com {
    owned: bool,
}

impl Com {
    fn enter() -> Self {
        // SAFETY: plain COM initialisation of this thread.
        let owned = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self { owned }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: balances the successful CoInitializeEx in `enter`.
            unsafe { CoUninitialize() };
        }
    }
}

/// `MFStartup` for the length of one encode.
struct Platform {
    shutdown: ShutdownFn,
}

impl Platform {
    fn start(api: &Api) -> Result<Self> {
        // SAFETY: balanced by `MFShutdown` in `drop`.
        let started = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_FULL) };
        done(started.ok(), "MFStartup")?;
        Ok(Self {
            shutdown: api.shutdown,
        })
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        // SAFETY: balances the successful MFStartup in `start`.
        let _ = unsafe { (self.shutdown)() };
    }
}

/// An audio media type at the archive's rate: PCM for what is written, AAC
/// for what the file holds.
fn media_type(
    api: &Api,
    subtype: &GUID,
    channels: u32,
    bytes_per_second: u32,
    block_alignment: u32,
) -> Result<IMFMediaType> {
    // SAFETY: the function was loaded for this signature.
    let media: IMFMediaType = created(
        |out| unsafe { (api.create_media_type)(out) },
        "MFCreateMediaType",
    )?;
    // SAFETY: plain calls on an object this function owns.
    unsafe {
        done(
            media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio),
            "media type",
        )?;
        done(media.SetGUID(&MF_MT_SUBTYPE, subtype), "media type")?;
        for (key, value) in [
            (&MF_MT_AUDIO_BITS_PER_SAMPLE, 16),
            (&MF_MT_AUDIO_SAMPLES_PER_SECOND, ENCODED_RATE),
            (&MF_MT_AUDIO_NUM_CHANNELS, channels),
            (&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_second),
            (&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_alignment),
        ] {
            done(media.SetUINT32(key, value), "media type")?;
        }
    }
    Ok(media)
}

/// One WAV track, read in blocks and brought up to 48 kHz.
struct Source {
    reader: hound::WavReader<BufReader<File>>,
    /// `None` once the end of the track went through.
    up: Option<Resampler>,
    block: Vec<f32>,
}

impl Source {
    fn open(path: &Path) -> Result<Self> {
        let reader =
            hound::WavReader::open(path).map_err(|_| anyhow!("Cannot open the meeting audio"))?;
        let spec = reader.spec();
        if spec.channels != 1
            || spec.sample_rate != TRACK_RATE
            || spec.bits_per_sample != 16
            || spec.sample_format != hound::SampleFormat::Int
        {
            bail!("The meeting audio has other tracks than expected");
        }
        Ok(Self {
            reader,
            up: Some(Resampler::up()),
            block: Vec::with_capacity(BLOCK),
        })
    }

    fn duration_ms(&self) -> u64 {
        u64::from(self.reader.duration()) * 1000 / u64::from(TRACK_RATE)
    }

    fn ended(&self) -> bool {
        self.up.is_none()
    }

    /// Replaces `out` with the next stretch of the track at 48 kHz.
    fn next(&mut self, out: &mut Vec<f32>) -> Result<()> {
        out.clear();
        let Some(up) = self.up.as_mut() else {
            return Ok(());
        };
        self.block.clear();
        for sample in self.reader.samples::<i16>().take(BLOCK) {
            let sample = sample.map_err(|_| anyhow!("Cannot read or write the meeting audio"))?;
            self.block.push(resample::from_i16(sample));
        }
        up.push(&self.block, out);
        if self.block.len() < BLOCK {
            if let Some(up) = self.up.take() {
                up.finish(out);
            }
        }
        Ok(())
    }
}

/// Where `frame` falls in the file, in 100 ns units.
fn hns(frame: u64) -> i64 {
    (frame * HNS_PER_SECOND / u64::from(ENCODED_RATE)) as i64
}

/// Hands `pcm` (`frames` frames starting at `first_frame`) to the writer.
fn write_pcm(
    api: &Api,
    writer: &IMFSinkWriter,
    stream: u32,
    pcm: &[u8],
    first_frame: u64,
    frames: u64,
) -> Result<()> {
    let length = u32::try_from(pcm.len()).map_err(|_| anyhow!("The audio block is too large"))?;
    // SAFETY: the function was loaded for this signature.
    let buffer: IMFMediaBuffer = created(
        |out| unsafe { (api.create_memory_buffer)(length, out) },
        "MFCreateMemoryBuffer",
    )?;
    let mut data: *mut u8 = std::ptr::null_mut();
    // SAFETY: the buffer holds `length` bytes, and is unlocked once copied.
    unsafe {
        done(buffer.Lock(&mut data, None, None), "buffer lock")?;
        std::ptr::copy_nonoverlapping(pcm.as_ptr(), data, pcm.len());
        done(buffer.Unlock(), "buffer unlock")?;
        done(buffer.SetCurrentLength(length), "buffer length")?;
    }
    // SAFETY: the function was loaded for this signature.
    let sample: IMFSample = created(|out| unsafe { (api.create_sample)(out) }, "MFCreateSample")?;
    // SAFETY: plain calls on objects this function owns.
    unsafe {
        done(sample.AddBuffer(&buffer), "sample buffer")?;
        done(sample.SetSampleTime(hns(first_frame)), "sample time")?;
        done(
            sample.SetSampleDuration(hns(first_frame + frames) - hns(first_frame)),
            "sample duration",
        )?;
        done(writer.WriteSample(stream, &sample), "write sample")
    }
}

/// Encodes `inputs` (one or two 16 kHz mono WAV tracks, the first on the
/// first channel) into the AAC file `output`, then decodes it to prove it
/// holds as much audio as the tracks. The shorter track is padded with
/// silence.
pub fn encode(inputs: &[PathBuf], output: &Path) -> Result<()> {
    if inputs.is_empty() || inputs.len() > 2 {
        bail!("A meeting has one or two tracks");
    }
    let mut sources = inputs
        .iter()
        .map(|path| Source::open(path))
        .collect::<Result<Vec<_>>>()?;
    let api = api()?;
    let expected_ms = sources.iter().map(Source::duration_ms).max().unwrap_or(0);
    let channels = sources.len() as u32;

    // Dropped last: the writer and the types go first.
    let _com = Com::enter();
    let _platform = Platform::start(api)?;

    let pcm_bytes = ENCODED_RATE * channels * 2;
    let input = media_type(api, &MFAudioFormat_PCM, channels, pcm_bytes, channels * 2)?;
    let target = media_type(api, &MFAudioFormat_AAC, channels, AAC_BYTES_PER_SECOND, 1)?;
    // SAFETY: plain calls on an object this function owns.
    unsafe {
        done(target.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0), "media type")?;
        done(
            target.SetUINT32(&MF_MT_AAC_AUDIO_PROFILE_LEVEL_INDICATION, AAC_PROFILE_LEVEL),
            "media type",
        )?;
    }

    // The part file's extension tells nothing: name the container.
    // SAFETY: the function was loaded for this signature.
    let attributes: IMFAttributes = created(
        |out| unsafe { (api.create_attributes)(out, 1) },
        "MFCreateAttributes",
    )?;
    // SAFETY: plain call on an object this function owns.
    unsafe {
        done(
            attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4),
            "container type",
        )?;
    }
    let _ = std::fs::remove_file(output);
    let url: Vec<u16> = output.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `url` is NUL-terminated and outlives the call; the byte stream
    // is null because the writer opens the file itself.
    let writer: IMFSinkWriter = created(
        |out| unsafe {
            (api.create_sink_writer)(
                PCWSTR(url.as_ptr()),
                std::ptr::null_mut(),
                attributes.as_raw(),
                out,
            )
        },
        "MFCreateSinkWriterFromURL",
    )?;
    // SAFETY: plain calls on an object this function owns. They fail when
    // Windows has no AAC encoder (N and KN editions without the Media Feature
    // Pack, Server without Media Foundation).
    let stream = unsafe {
        let stream = done(writer.AddStream(&target), "AAC stream")?;
        done(
            writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>),
            "AAC encoder",
        )?;
        done(writer.BeginWriting(), "begin writing")?;
        stream
    };

    let mut lanes: Vec<Vec<f32>> = vec![Vec::new(); sources.len()];
    let mut pcm = Vec::new();
    let mut written = 0u64;
    loop {
        for (source, lane) in sources.iter_mut().zip(&mut lanes) {
            source.next(lane)?;
        }
        let frames = lanes.iter().map(Vec::len).max().unwrap_or(0);
        if frames == 0 {
            if sources.iter().all(Source::ended) {
                break;
            }
            continue;
        }
        resample::interleave_pcm16(&lanes, frames, &mut pcm);
        write_pcm(api, &writer, stream, &pcm, written, frames as u64)?;
        written += frames as u64;
    }
    // SAFETY: plain call; the file is complete once it returns.
    unsafe { done(writer.Finalize(), "finalize")? };
    drop(writer);

    let decoded_ms = aac::decoded_ms(output)?;
    if decoded_ms.abs_diff(expected_ms) > DURATION_TOLERANCE_MS {
        bail!("The compressed audio decodes to {decoded_ms} ms instead of {expected_ms} ms");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_fall_on_the_clock_without_drift() {
        assert_eq!(hns(0), 0);
        assert_eq!(hns(48_000), 10_000_000);
        // One AAC frame of 1024 samples at 48 kHz.
        assert_eq!(hns(1_024), 213_333);
        // 3 hours, to the sample.
        let three_hours = 3 * 3_600 * u64::from(ENCODED_RATE);
        assert_eq!(hns(three_hours), 3 * 3_600 * 10_000_000);
    }

    #[test]
    fn a_meeting_has_one_or_two_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.m4a.part");
        assert!(encode(&[], &output).is_err());
        let three = [PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c")];
        assert!(encode(&three, &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn a_track_at_another_rate_is_refused_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let track = dir.path().join("mic.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&track, spec).unwrap();
        writer.write_sample(0i16).unwrap();
        writer.finalize().unwrap();
        let output = dir.path().join("out.m4a.part");
        assert!(encode(&[track], &output).is_err());
        assert!(!output.exists());
    }
}
