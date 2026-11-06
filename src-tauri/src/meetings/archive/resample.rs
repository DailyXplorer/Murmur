//! Rate conversion between the 16 kHz meeting tracks and the 48 kHz the
//! Windows AAC encoder takes (it accepts 44.1 and 48 kHz only). Both
//! directions are a factor of 3, so one polyphase FIR does either: a
//! windowed-sinc kernel, symmetric, so the output has no delay and the
//! lengths are exact (3 samples out per sample in, or 1 per 3). It streams:
//! a 3-hour track never sits in memory whole.

use std::f64::consts::PI;

/// The rate of a meeting's WAV tracks.
pub const TRACK_RATE: u32 = 16_000;
/// The rate the archive's AAC is encoded at.
pub const ENCODED_RATE: u32 = 48_000;
const FACTOR: usize = 3;

/// A 16-bit sample as a float in `-1..1`.
pub fn from_i16(sample: i16) -> f32 {
    f32::from(sample) / 32_768.0
}

/// A float sample as 16 bits, clipped.
pub fn to_i16(sample: f32) -> i16 {
    (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16
}

/// Packs `lanes` (one per channel) as interleaved 16-bit little-endian PCM
/// of `frames` frames, a lane shorter than that padded with silence.
pub fn interleave_pcm16(lanes: &[Vec<f32>], frames: usize, out: &mut Vec<u8>) {
    out.clear();
    for frame in 0..frames {
        for lane in lanes {
            let sample = lane.get(frame).copied().unwrap_or(0.0);
            out.extend_from_slice(&to_i16(sample).to_le_bytes());
        }
    }
}

/// Input samples on each side of an interpolated point.
const UP_SIDE: usize = 16;
/// Output samples on each side of a decimated point.
const DOWN_SIDE: usize = 16;
/// Where the decimating filter cuts, as a share of the 8 kHz that the 16 kHz
/// output can hold.
const DOWN_CUTOFF: f64 = 0.92;

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Blackman window over `-1..=1`, zero outside.
fn window(x: f64) -> f64 {
    if x.abs() >= 1.0 {
        0.0
    } else {
        0.42 + 0.5 * (PI * x).cos() + 0.08 * (2.0 * PI * x).cos()
    }
}

/// Scales `taps` to a sum of 1, so a constant signal keeps its level.
fn normalized(taps: Vec<f64>) -> Vec<f32> {
    let sum: f64 = taps.iter().sum();
    taps.into_iter().map(|tap| (tap / sum) as f32).collect()
}

/// One conversion from 16 kHz to 48 kHz or back, fed in blocks of any size.
pub struct Resampler {
    /// One kernel per output sample produced for each step.
    phases: Vec<Vec<f32>>,
    /// Input samples consumed per step.
    advance: usize,
    /// Samples a kernel reads before and after its centre.
    before: usize,
    after: usize,
    buffer: Vec<f32>,
    /// Index in `buffer` of the centre of the next step.
    centre: usize,
}

impl Resampler {
    /// 16 kHz to 48 kHz: three outputs for each input.
    pub fn up() -> Self {
        // Output `phase` of input `n` sits `phase / 3` after it and reads the
        // inputs `n + 1 - UP_SIDE` through `n + UP_SIDE`.
        let phases = (0..FACTOR)
            .map(|phase| {
                normalized(
                    (0..2 * UP_SIDE)
                        .map(|index| {
                            let offset = index as f64 + 1.0 - UP_SIDE as f64;
                            let distance = phase as f64 / FACTOR as f64 - offset;
                            sinc(distance) * window(distance / UP_SIDE as f64)
                        })
                        .collect(),
                )
            })
            .collect();
        Self::new(phases, 1, UP_SIDE - 1, UP_SIDE)
    }

    /// 48 kHz to 16 kHz: one output for each three inputs.
    pub fn down() -> Self {
        let half = FACTOR * DOWN_SIDE;
        let kernel = (0..=2 * half)
            .map(|index| {
                let offset = index as f64 - half as f64;
                let scaled = DOWN_CUTOFF * offset / FACTOR as f64;
                sinc(scaled) * window(offset / half as f64)
            })
            .collect();
        Self::new(vec![normalized(kernel)], FACTOR, half, half)
    }

    fn new(phases: Vec<Vec<f32>>, advance: usize, before: usize, after: usize) -> Self {
        Self {
            phases,
            advance,
            before,
            after,
            buffer: vec![0.0; before],
            centre: before,
        }
    }

    /// Appends the samples that `input` completes to `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buffer.extend_from_slice(input);
        while self.centre + self.after < self.buffer.len() {
            let window = &self.buffer[self.centre - self.before..=self.centre + self.after];
            for taps in &self.phases {
                out.push(taps.iter().zip(window).map(|(tap, x)| tap * x).sum());
            }
            self.centre += self.advance;
        }
        let spent = self.centre - self.before;
        self.buffer.drain(..spent);
        self.centre -= spent;
    }

    /// Appends the samples the end of the signal was holding back.
    pub fn finish(mut self, out: &mut Vec<f32>) {
        let silence = vec![0.0; self.after];
        self.push(&silence, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 16_000.0;

    fn sine(hz: f64, rate: f64, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|index| (2.0 * PI * hz * index as f64 / rate).sin() as f32 * 0.5)
            .collect()
    }

    fn convert(mut resampler: Resampler, input: &[f32], block: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for piece in input.chunks(block) {
            resampler.push(piece, &mut out);
        }
        resampler.finish(&mut out);
        out
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn channels_interleave_and_the_shorter_one_is_padded() {
        let lanes = [vec![0.5, -0.5, 0.25], vec![1.0]];
        let mut out = Vec::new();
        interleave_pcm16(&lanes, 3, &mut out);
        let samples: Vec<i16> = out
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        // 1.0 clips to the largest 16-bit value.
        assert_eq!(samples, [16_384, 32_767, -16_384, 0, 8_192, 0]);
    }

    #[test]
    fn samples_round_trip_between_16_bits_and_floats() {
        for sample in [i16::MIN, -1, 0, 1, 12_345, i16::MAX] {
            assert_eq!(to_i16(from_i16(sample)), sample);
        }
        assert_eq!(to_i16(3.0), i16::MAX);
        assert_eq!(to_i16(-3.0), i16::MIN);
    }

    #[test]
    fn lengths_are_exact_for_any_input_length() {
        for length in [0, 1, 2, 3, 100, 4_097] {
            let input = vec![0.1; length];
            assert_eq!(convert(Resampler::up(), &input, 4_096).len(), length * 3);
            assert_eq!(
                convert(Resampler::down(), &input, 4_096).len(),
                length.div_ceil(3)
            );
        }
    }

    #[test]
    fn block_sizes_do_not_change_the_result() {
        let input = sine(440.0, RATE, 10_000);
        let whole = convert(Resampler::up(), &input, input.len());
        for block in [1, 7, 333, 4_096] {
            assert_eq!(convert(Resampler::up(), &input, block), whole, "{block}");
        }
        let fast = sine(440.0, RATE * 3.0, 30_000);
        let whole = convert(Resampler::down(), &fast, fast.len());
        for block in [1, 7, 333, 4_096] {
            assert_eq!(convert(Resampler::down(), &fast, block), whole, "{block}");
        }
    }

    #[test]
    fn going_up_keeps_the_original_samples() {
        let input = sine(1_000.0, RATE, 2_000);
        let up = convert(Resampler::up(), &input, 512);
        for (index, sample) in input.iter().enumerate() {
            assert!((up[index * 3] - sample).abs() < 1e-5, "{index}");
        }
    }

    #[test]
    fn a_constant_level_survives_both_ways() {
        let up = convert(Resampler::up(), &[0.25; 2_000], 512);
        assert!(up[100..up.len() - 100]
            .iter()
            .all(|x| (x - 0.25).abs() < 1e-5));
        let down = convert(Resampler::down(), &[0.25; 6_000], 512);
        assert!(down[100..down.len() - 100]
            .iter()
            .all(|x| (x - 0.25).abs() < 1e-5));
    }

    #[test]
    fn a_tone_comes_back_in_phase_and_level() {
        let input = sine(1_000.0, RATE, 16_000);
        let back = convert(
            Resampler::down(),
            &convert(Resampler::up(), &input, 4_096),
            4_096,
        );
        assert_eq!(back.len(), input.len());
        let error: Vec<f32> = input
            .iter()
            .zip(&back)
            .skip(200)
            .take(15_000)
            .map(|(a, b)| a - b)
            .collect();
        assert!(rms(&error) < 0.005 * rms(&input), "{}", rms(&error));
    }

    #[test]
    fn going_down_removes_what_the_lower_rate_cannot_hold() {
        // 12 kHz at 48 kHz would fold to 4 kHz without the filter.
        let high = sine(12_000.0, RATE * 3.0, 48_000);
        let down = convert(Resampler::down(), &high, 4_096);
        assert!(rms(&down[200..]) < 0.001 * 0.5, "{}", rms(&down[200..]));
        // 5 kHz is inside the band.
        let inside = sine(5_000.0, RATE * 3.0, 48_000);
        let down = convert(Resampler::down(), &inside, 4_096);
        assert!(rms(&down[200..down.len() - 200]) > 0.9 * rms(&inside));
    }
}
