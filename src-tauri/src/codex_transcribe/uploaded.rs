//! For the fake Codex servers of the tests: the audio of an upload as 16-bit
//! samples, whether it came as FLAC or as WAV.

use super::SAMPLE_RATE;

/// The samples of the audio file in a multipart request `body`. `None` when
/// no part of it is FLAC or WAV.
pub(crate) fn pcm_of_request(body: &[u8]) -> Option<Vec<i16>> {
    let boundary = body.split(|byte| *byte == b'\r').next()?;
    let mut rest = body;
    while let Some(start) = find(rest, boundary) {
        rest = &rest[start + boundary.len()..];
        let part = &rest[..find(rest, boundary).unwrap_or(rest.len())];
        let content = find(part, b"\r\n\r\n").map(|headers| &part[headers + 4..]);
        if let Some(samples) = content.and_then(pcm) {
            return Some(samples);
        }
    }
    None
}

fn find(bytes: &[u8], wanted: &[u8]) -> Option<usize> {
    bytes
        .windows(wanted.len())
        .position(|window| window == wanted)
}

/// The samples of a FLAC or WAV file.
pub(crate) fn pcm(audio: &[u8]) -> Option<Vec<i16>> {
    if audio.starts_with(b"fLaC") {
        Some(flac(audio))
    } else if audio.starts_with(b"RIFF") {
        Some(wav(audio))
    } else {
        None
    }
}

fn wav(audio: &[u8]) -> Vec<i16> {
    hound::WavReader::new(std::io::Cursor::new(audio))
        .unwrap()
        .samples::<i16>()
        .map(Result::unwrap)
        .collect()
}

/// Decoded with symphonia, which refuses what a lenient decoder lets through.
fn flac(audio: &[u8]) -> Vec<i16> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::probe::Hint;

    let stream = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(audio.to_vec())),
        Default::default(),
    );
    let mut format = symphonia::default::get_probe()
        .format(
            Hint::new().with_extension("flac"),
            stream,
            &Default::default(),
            &Default::default(),
        )
        .unwrap()
        .format;
    let track = format.default_track().unwrap();
    assert_eq!(track.codec_params.sample_rate, Some(SAMPLE_RATE));
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &Default::default())
        .unwrap();
    let mut samples = Vec::new();
    while let Ok(packet) = format.next_packet() {
        let decoded = decoder.decode(&packet).unwrap();
        let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    samples
}
