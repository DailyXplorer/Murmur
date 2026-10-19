//! A processed meeting keeps its audio as one small AAC file, `audio.m4a`
//! (about 29 MB an hour), instead of its WAV tracks (about 115 MB an hour
//! each): the microphone on the first channel and system audio on the
//! second. Transcribing the meeting again decodes the tracks back. The work
//! happens in `swift/meeting_archive.swift`.

#[cfg(unix)]
use anyhow::anyhow;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use super::paths::{self, AUDIO_FILE, MIC_TRACK, SYSTEM_TRACK};
use super::store::{AudioState, MeetingTracks, MeetingsStore};

/// AAC frames and the encoder's padding can make the compressed audio a
/// little longer or shorter than the tracks.
const DURATION_TOLERANCE_MS: u64 = 1_000;

#[cfg(target_arch = "aarch64")]
mod bridge {
    use std::ffi::c_char;

    extern "C" {
        pub fn murmur_encode_m4a(
            first: *const c_char,
            second: *const c_char,
            output: *const c_char,
        ) -> i32;
        pub fn murmur_decode_m4a(
            input: *const c_char,
            first: *const c_char,
            second: *const c_char,
        ) -> i32;
        pub fn murmur_audio_duration_ms(path: *const c_char) -> i64;
    }
}

/// Mirrors the status codes in `swift/meeting_archive.swift`.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
fn status(code: i32) -> Result<()> {
    match code {
        0 => Ok(()),
        1 => bail!("Cannot open the meeting audio"),
        2 => bail!("Cannot create the meeting audio file"),
        3 => bail!("Cannot read or write the meeting audio"),
        4 => bail!("The meeting audio has other tracks than expected"),
        _ => bail!("Meeting audio conversion failed"),
    }
}

#[cfg(unix)]
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
fn c_path(path: &Path) -> Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| anyhow!("The meeting path has a NUL byte"))
}

/// The tracks a meeting recorded, in channel order.
pub fn track_files(tracks: MeetingTracks) -> Vec<&'static str> {
    [
        (tracks.has_mic(), MIC_TRACK),
        (tracks.has_system(), SYSTEM_TRACK),
    ]
    .into_iter()
    .filter_map(|(recorded, file)| recorded.then_some(file))
    .collect()
}

/// The length of an audio file in milliseconds.
pub fn duration_ms(path: &Path) -> Result<u64> {
    #[cfg(target_arch = "aarch64")]
    {
        let path = c_path(path)?;
        // SAFETY: `path` is a NUL-terminated string alive for the call.
        let duration = unsafe { bridge::murmur_audio_duration_ms(path.as_ptr()) };
        u64::try_from(duration).map_err(|_| anyhow!("Cannot read the meeting audio length"))
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = path;
        bail!("Compressed meeting audio needs Apple silicon")
    }
}

/// Encodes `inputs` (one or two WAV tracks) into the AAC file `output`.
pub fn encode(inputs: &[PathBuf], output: &Path) -> Result<()> {
    if inputs.is_empty() || inputs.len() > 2 {
        bail!("A meeting has one or two tracks");
    }
    #[cfg(target_arch = "aarch64")]
    {
        let first = c_path(&inputs[0])?;
        let second = inputs.get(1).map(|path| c_path(path)).transpose()?;
        let output = c_path(output)?;
        // SAFETY: every pointer is a NUL-terminated string alive for the
        // call, or null for a missing second track.
        status(unsafe {
            bridge::murmur_encode_m4a(
                first.as_ptr(),
                second
                    .as_ref()
                    .map_or(std::ptr::null(), |path| path.as_ptr()),
                output.as_ptr(),
            )
        })
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = output;
        bail!("Compressed meeting audio needs Apple silicon")
    }
}

/// Decodes the AAC file `input` into one WAV per channel, `outputs` in
/// channel order.
pub fn decode(input: &Path, outputs: &[PathBuf]) -> Result<()> {
    if outputs.is_empty() || outputs.len() > 2 {
        bail!("A meeting has one or two tracks");
    }
    #[cfg(target_arch = "aarch64")]
    {
        let input = c_path(input)?;
        let first = c_path(&outputs[0])?;
        let second = outputs.get(1).map(|path| c_path(path)).transpose()?;
        // SAFETY: as in `encode`.
        status(unsafe {
            bridge::murmur_decode_m4a(
                input.as_ptr(),
                first.as_ptr(),
                second
                    .as_ref()
                    .map_or(std::ptr::null(), |path| path.as_ptr()),
            )
        })
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = input;
        bail!("Compressed meeting audio needs Apple silicon")
    }
}

fn wav_duration_ms(path: &Path) -> Result<u64> {
    let reader = hound::WavReader::open(path)?;
    Ok(u64::from(reader.duration()) * 1000 / u64::from(reader.spec().sample_rate))
}

/// Compresses the tracks in `dir` into `audio.m4a` and checks it holds as
/// much audio. Returns its size.
pub fn compress(dir: &Path, tracks: MeetingTracks) -> Result<u64> {
    let inputs: Vec<PathBuf> = track_files(tracks)
        .into_iter()
        .map(|file| dir.join(file))
        .collect();
    let expected = inputs
        .iter()
        .map(|path| wav_duration_ms(path))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    let partial = dir.join(paths::partial(AUDIO_FILE));
    let output = dir.join(AUDIO_FILE);
    let written = encode(&inputs, &partial).and_then(|()| {
        let actual = duration_ms(&partial)?;
        if actual.abs_diff(expected) > DURATION_TOLERANCE_MS {
            bail!("The compressed audio lasts {actual} ms instead of {expected} ms");
        }
        Ok(std::fs::rename(&partial, &output)?)
    });
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written?;
    Ok(std::fs::metadata(output)?.len())
}

/// Brings back the tracks of `dir` that are missing, from `audio.m4a`.
/// Does nothing when every track is there or there is no compressed audio.
pub fn restore(dir: &Path, tracks: MeetingTracks) -> Result<()> {
    let files = track_files(tracks);
    let source = dir.join(AUDIO_FILE);
    if files.iter().all(|file| dir.join(file).exists()) || !source.exists() {
        return Ok(());
    }
    let partials: Vec<PathBuf> = files
        .iter()
        .map(|file| dir.join(paths::partial(file)))
        .collect();
    let restored = decode(&source, &partials).and_then(|()| {
        for (partial, file) in partials.iter().zip(&files) {
            std::fs::rename(partial, dir.join(file))?;
        }
        Ok(())
    });
    if restored.is_err() {
        for partial in &partials {
            let _ = std::fs::remove_file(partial);
        }
    }
    restored
}

/// Stage E: compresses the audio of a transcribed meeting once, then
/// deletes its tracks. The tracks stay when compression fails, and it is
/// tried again at the next launch.
pub fn archive(store: &MeetingsStore, root: &Path, id: i64) -> Result<()> {
    let Some(meeting) = store.get(id)? else {
        return Ok(());
    };
    let dir = paths::meeting_dir(root, id);
    let files = track_files(meeting.tracks);
    match meeting.audio {
        AudioState::Deleted => return Ok(()),
        AudioState::Compressed => {}
        AudioState::Tracks => {
            if !files.iter().any(|file| dir.join(file).exists()) {
                return Ok(());
            }
            let bytes = compress(&dir, meeting.tracks)?;
            if store.audio_compressed(id, bytes)? {
                log::info!(
                    "Meeting {id}: audio compressed to {} KB",
                    bytes.div_ceil(1024)
                );
            } else {
                // The user deleted the audio meanwhile.
                paths::remove_meeting_files(root, id, &[AUDIO_FILE])?;
            }
        }
    }
    paths::remove_meeting_files(root, id, &files)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_go_in_channel_order() {
        assert_eq!(track_files(MeetingTracks::Both), [MIC_TRACK, SYSTEM_TRACK]);
        assert_eq!(track_files(MeetingTracks::System), [SYSTEM_TRACK]);
        assert_eq!(track_files(MeetingTracks::Mic), [MIC_TRACK]);
    }

    #[cfg(target_arch = "aarch64")]
    mod on_device {
        use super::*;
        use crate::meetings::store::{MeetingStatus, MeetingTracks};

        const RATE: u32 = 16_000;

        /// A tone at `hz` for `seconds`.
        fn tone(path: &Path, seconds: u32, hz: f32) {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(path, spec).unwrap();
            for index in 0..seconds * RATE {
                let phase = index as f32 * hz * std::f32::consts::TAU / RATE as f32;
                writer.write_sample((phase.sin() * 8_000.0) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }

        fn samples(path: &Path) -> Vec<i16> {
            hound::WavReader::open(path)
                .unwrap()
                .into_samples::<i16>()
                .map(Result::unwrap)
                .collect()
        }

        /// Loudness of the 440 Hz and 1 kHz tones in `samples`, to tell the
        /// channels apart after a lossy round trip.
        fn energy_at(samples: &[i16], hz: f32) -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (index, sample) in samples.iter().enumerate() {
                let phase = index as f32 * hz * std::f32::consts::TAU / RATE as f32;
                re += *sample as f32 * phase.cos();
                im += *sample as f32 * phase.sin();
            }
            (re * re + im * im).sqrt() / samples.len() as f32
        }

        #[test]
        fn two_tracks_round_trip_through_a_small_file() {
            let dir = tempfile::tempdir().unwrap();
            // The system track is a second shorter: it is padded.
            tone(&dir.path().join(MIC_TRACK), 60, 440.0);
            tone(&dir.path().join(SYSTEM_TRACK), 59, 1_000.0);

            let bytes = compress(dir.path(), MeetingTracks::Both).unwrap();
            // At most 35 MB an hour.
            assert!(bytes <= 35_000_000 / 60, "{bytes} bytes for one minute");
            let duration = duration_ms(&dir.path().join(AUDIO_FILE)).unwrap();
            assert!(duration.abs_diff(60_000) <= 200, "{duration} ms");

            std::fs::remove_file(dir.path().join(MIC_TRACK)).unwrap();
            std::fs::remove_file(dir.path().join(SYSTEM_TRACK)).unwrap();
            restore(dir.path(), MeetingTracks::Both).unwrap();
            let mic = samples(&dir.path().join(MIC_TRACK));
            let system = samples(&dir.path().join(SYSTEM_TRACK));
            assert!((mic.len() as i64 - 60 * RATE as i64).abs() < RATE as i64 / 10);
            assert_eq!(mic.len(), system.len());
            // Each channel came back to its own track.
            let middle = |track: &[i16]| track[RATE as usize * 10..RATE as usize * 20].to_vec();
            assert!(energy_at(&middle(&mic), 440.0) > 10.0 * energy_at(&middle(&mic), 1_000.0));
            assert!(
                energy_at(&middle(&system), 1_000.0) > 10.0 * energy_at(&middle(&system), 440.0)
            );
        }

        #[test]
        fn a_single_track_round_trips_and_restoring_leaves_existing_tracks() {
            let dir = tempfile::tempdir().unwrap();
            tone(&dir.path().join(SYSTEM_TRACK), 5, 440.0);
            compress(dir.path(), MeetingTracks::System).unwrap();
            let before = std::fs::metadata(dir.path().join(SYSTEM_TRACK)).unwrap();
            restore(dir.path(), MeetingTracks::System).unwrap();
            let after = std::fs::metadata(dir.path().join(SYSTEM_TRACK)).unwrap();
            assert_eq!(before.modified().unwrap(), after.modified().unwrap());

            std::fs::remove_file(dir.path().join(SYSTEM_TRACK)).unwrap();
            restore(dir.path(), MeetingTracks::System).unwrap();
            let length = samples(&dir.path().join(SYSTEM_TRACK)).len() as i64;
            assert!((length - 5 * RATE as i64).abs() < RATE as i64 / 10);
            // A file of one channel cannot fill two tracks.
            std::fs::remove_file(dir.path().join(SYSTEM_TRACK)).unwrap();
            assert!(restore(dir.path(), MeetingTracks::Both).is_err());
        }

        #[test]
        fn archiving_a_ready_meeting_keeps_only_the_compressed_audio() {
            let dir = tempfile::tempdir().unwrap();
            let (store, _) =
                MeetingsStore::open(dir.path().join("meetings.db"), chrono::Utc::now());
            let root = dir.path().join("meetings");
            let id = store.insert_recording(0, MeetingTracks::Mic).unwrap();
            store.finish_recording(id, 3_000, 3_000).unwrap();
            store.begin_processing(id).unwrap();
            store.finish_processing(id, None).unwrap();
            let meeting_dir = paths::meeting_dir(&root, id);
            std::fs::create_dir_all(&meeting_dir).unwrap();
            tone(&meeting_dir.join(MIC_TRACK), 3, 440.0);

            archive(&store, &root, id).unwrap();
            let meeting = store.get(id).unwrap().unwrap();
            assert_eq!(meeting.status, MeetingStatus::Ready);
            assert_eq!(meeting.audio, AudioState::Compressed);
            assert!(meeting.audio_bytes > 0);
            assert!(meeting_dir.join(AUDIO_FILE).exists());
            assert!(!meeting_dir.join(MIC_TRACK).exists());
            // Again: nothing to do.
            archive(&store, &root, id).unwrap();
            assert!(meeting_dir.join(AUDIO_FILE).exists());
        }

        #[test]
        fn a_track_that_cannot_be_read_is_kept() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(MIC_TRACK), b"not audio").unwrap();
            assert!(compress(dir.path(), MeetingTracks::Mic).is_err());
            assert!(dir.path().join(MIC_TRACK).exists());
            assert!(!dir.path().join(AUDIO_FILE).exists());
        }

        #[test]
        fn a_failed_restore_leaves_no_half_written_track() {
            let dir = tempfile::tempdir().unwrap();
            tone(&dir.path().join(SYSTEM_TRACK), 2, 440.0);
            compress(dir.path(), MeetingTracks::System).unwrap();
            std::fs::remove_file(dir.path().join(SYSTEM_TRACK)).unwrap();
            // One channel cannot fill two tracks.
            assert!(restore(dir.path(), MeetingTracks::Both).is_err());
            std::fs::write(dir.path().join(AUDIO_FILE), b"not audio").unwrap();
            assert!(restore(dir.path(), MeetingTracks::System).is_err());

            let left: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(left, [AUDIO_FILE]);
        }
    }
}
