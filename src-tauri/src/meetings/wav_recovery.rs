//! Repairs a track WAV that a crash left with stale sizes in its header. The
//! writer flushes the header every few seconds, so after a crash the samples
//! written since the last flush are on disk but not counted; the real length
//! is whatever the file holds after the `data` chunk header.

use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Rewrites the RIFF and `data` sizes from the file length, drops a trailing
/// partial sample, and returns the number of sample frames now in the file.
pub fn repair_wav(path: &Path) -> io::Result<u64> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_len = file.metadata()?.len();
    let mut header = vec![0u8; file_len.min(4096) as usize];
    file.read_exact(&mut header)?;
    let layout = parse_layout(&header).ok_or_else(|| invalid("not a PCM WAV file"))?;

    let data_len = file_len.saturating_sub(layout.data_start);
    let data_len = data_len - data_len % u64::from(layout.block_align);
    let riff_size = u32::try_from(layout.data_start + data_len - 8)
        .map_err(|_| invalid("WAV file is larger than 4 GB"))?;
    let data_size = u32::try_from(data_len).map_err(|_| invalid("WAV data is larger than 4 GB"))?;

    file.set_len(layout.data_start + data_len)?;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.seek(SeekFrom::Start(layout.data_start - 4))?;
    file.write_all(&data_size.to_le_bytes())?;
    file.sync_all()?;
    Ok(data_len / u64::from(layout.block_align))
}

struct Layout {
    data_start: u64,
    block_align: u16,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Walks the chunks after `RIFF....WAVE` up to `data`. The `data` size is
/// the one value not trusted, since it is what the crash left stale.
fn parse_layout(header: &[u8]) -> Option<Layout> {
    if header.get(0..4)? != b"RIFF" || header.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut block_align = None;
    let mut position = 12usize;
    loop {
        let id = header.get(position..position + 4)?;
        let size = u32::from_le_bytes(header.get(position + 4..position + 8)?.try_into().ok()?);
        let body = position + 8;
        if id == b"data" {
            return Some(Layout {
                data_start: body as u64,
                block_align: block_align.filter(|align| *align > 0)?,
            });
        }
        if id == b"fmt " {
            block_align = Some(u16::from_le_bytes(
                header.get(body + 12..body + 14)?.try_into().ok()?,
            ));
        }
        // Chunks are padded to an even length.
        position = body.checked_add(size as usize + (size as usize & 1))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> hound::WavSpec {
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        }
    }

    /// A WAV as a crash leaves it: the header was last flushed after
    /// `counted` samples, but `written` samples reached the disk, plus half
    /// of one more.
    fn crashed_wav(counted: usize, written: usize) -> Vec<u8> {
        let mut bytes = io::Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut bytes, spec()).unwrap();
        for index in 0..counted {
            writer.write_sample(index as i16).unwrap();
        }
        writer.finalize().unwrap();
        let mut bytes = bytes.into_inner();
        for index in counted..written {
            bytes.extend_from_slice(&(index as i16).to_le_bytes());
        }
        bytes.push(0x7f);
        bytes
    }

    #[test]
    fn a_truncated_header_is_repaired_from_the_file_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.wav");
        std::fs::write(&path, crashed_wav(1_000, 16_000)).unwrap();

        assert_eq!(repair_wav(&path).unwrap(), 16_000);

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec(), spec());
        assert_eq!(reader.duration(), 16_000);
        let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 16_000);
        assert_eq!(samples[15_999], 15_999);
    }

    #[test]
    fn a_header_with_no_samples_counted_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        std::fs::write(&path, crashed_wav(0, 320)).unwrap();

        assert_eq!(repair_wav(&path).unwrap(), 320);
        assert_eq!(hound::WavReader::open(&path).unwrap().duration(), 320);
    }

    #[test]
    fn a_healthy_file_is_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.wav");
        let mut bytes = crashed_wav(500, 500);
        bytes.pop();
        std::fs::write(&path, &bytes).unwrap();

        assert_eq!(repair_wav(&path).unwrap(), 500);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn a_file_that_is_not_a_wav_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.wav");
        std::fs::write(&path, b"not audio at all").unwrap();
        assert_eq!(
            repair_wav(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
