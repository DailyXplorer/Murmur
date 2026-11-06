//! The log-mel features the speaker encoder reads, computed the way the
//! `FBank` Core ML graph of FluidAudio 0.17.7 does (`mil/fbank.mil`, built
//! from `torchaudio.compliance.kaldi.fbank`), plus the re-centring of
//! `SilenceAwareFbank.swift`. Plain Rust on slices, so the numbers can be
//! checked on any platform.
//!
//! One window of 160000 samples gives `FBANK_FRAMES x MEL_BINS` values, row
//! major (frame, then mel bin): the layout of the ONNX encoder input
//! `[B, 998, 80]`.
//!
//! The graph, per frame of 400 samples taken every 160 with no padding:
//! samples x 32768, minus the frame mean, pre-emphasis `x[i] - 0.97 x[i-1]`
//! with `x[-1] = x[0]`, symmetric Hamming window, zero-padded to 512, power
//! spectrum (257 bins), 80 Kaldi mel bins, `ln(max(mel + 2^-23, 2^-23))`, and
//! finally the mean over the frames removed from every bin. The x 32768 is
//! not cosmetic: it puts the power spectrum on the scale where the 2^-23
//! floor is negligible for speech and bites only on digital silence.

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

use super::config::{FBANK_FRAMES, MEL_BINS, SAMPLE_RATE, WINDOW_SAMPLES};

/// `fbank.mil`, `frames_1`: a convolution of kernel 400, stride 160.
const FRAME_LENGTH: usize = 400;
const FRAME_SHIFT: usize = 160;
/// `fbank.mil`, `op_67`: frames are zero-padded from 400 to 512 samples.
const FFT_SIZE: usize = 512;
/// Bins of the one-sided spectrum of a 512-point transform.
const SPECTRUM_BINS: usize = FFT_SIZE / 2 + 1;
/// `fbank.mil`, `op_3_promoted`: 2^15, the 16-bit full scale.
const INPUT_SCALE: f32 = 32768.0;
/// `fbank.mil`, `op_61` (0x1.f0a3d8p-1, the f32 nearest 0.97).
const PRE_EMPHASIS: f32 = 0.97;
/// `fbank.mil`, `_fbank_eps` (0x1.0c6f7ap-20 is 2^-23): added to the mel
/// energies and the floor of their clip, so digital silence gives `ln(2^-23)`.
const LOG_FLOOR: f32 = f32::EPSILON;
/// Lowest edge of the mel filters (Kaldi `low_freq`).
const LOW_FREQ: f64 = 20.0;

/// One triangular mel filter: the weights of the spectrum bins from `first`.
struct MelBin {
    first: usize,
    weights: Vec<f32>,
}

/// Everything that does not depend on the audio, built once.
pub struct Fbank {
    window: Vec<f32>,
    mel: Vec<MelBin>,
    fft: Arc<dyn Fft<f32>>,
}

impl Default for Fbank {
    fn default() -> Self {
        Self::new()
    }
}

impl Fbank {
    pub fn new() -> Fbank {
        Fbank {
            window: hamming_window(),
            mel: mel_filters(),
            fft: FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE),
        }
    }

    /// The features of one window of audio in `[-1, 1]`: `FBANK_FRAMES x
    /// MEL_BINS` values, frame by frame. Audio longer than `WINDOW_SAMPLES` is
    /// cut and shorter audio is zero-padded, as `prepareFbankInput` does
    /// (`OfflineEmbeddingExtractor.swift` line 837): the padding is digital
    /// silence and is left out of the mean (see `centre`).
    pub fn window(&self, audio: &[f32]) -> Vec<f32> {
        let mut padded: Vec<f32> = vec![0.0; WINDOW_SAMPLES];
        let used: usize = audio.len().min(WINDOW_SAMPLES);
        padded[..used].copy_from_slice(&audio[..used]);
        let (mut features, silent) = self.log_mel(&padded);
        centre(&mut features, &silent);
        features
    }

    /// Log-mel energies of every frame before the mean is removed, and which
    /// frames are exact digital silence. `padded` holds `WINDOW_SAMPLES`.
    fn log_mel(&self, padded: &[f32]) -> (Vec<f32>, Vec<bool>) {
        let mut features: Vec<f32> = vec![0.0; FBANK_FRAMES * MEL_BINS];
        let mut silent: Vec<bool> = vec![false; FBANK_FRAMES];
        let mut buffer: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); FFT_SIZE];
        let mut scratch: Vec<Complex<f32>> =
            vec![Complex::new(0.0, 0.0); self.fft.get_inplace_scratch_len()];
        let mut power: Vec<f32> = vec![0.0; SPECTRUM_BINS];
        let mut shaped: Vec<f32> = vec![0.0; FRAME_LENGTH];

        for (frame_index, row) in features
            .as_chunks_mut::<MEL_BINS>()
            .0
            .iter_mut()
            .enumerate()
        {
            let start: usize = frame_index * FRAME_SHIFT;
            let samples: &[f32] = &padded[start..start + FRAME_LENGTH];
            if samples.iter().all(|&sample| sample == 0.0) {
                silent[frame_index] = true;
                row.fill(LOG_FLOOR.ln());
                continue;
            }

            let mean: f32 = (samples
                .iter()
                .map(|&sample| f64::from(sample * INPUT_SCALE))
                .sum::<f64>()
                / FRAME_LENGTH as f64) as f32;
            for (slot, &sample) in shaped.iter_mut().zip(samples) {
                *slot = sample * INPUT_SCALE - mean;
            }
            for (index, slot) in buffer.iter_mut().enumerate() {
                *slot = if index < FRAME_LENGTH {
                    let previous: f32 = shaped[index.saturating_sub(1)];
                    let emphasised: f32 = shaped[index] - PRE_EMPHASIS * previous;
                    Complex::new(emphasised * self.window[index], 0.0)
                } else {
                    Complex::new(0.0, 0.0)
                };
            }
            self.fft.process_with_scratch(&mut buffer, &mut scratch);
            for (energy, bin) in power.iter_mut().zip(buffer.iter()) {
                *energy = bin.re * bin.re + bin.im * bin.im;
            }
            for (value, filter) in row.iter_mut().zip(&self.mel) {
                let energy: f32 = filter
                    .weights
                    .iter()
                    .zip(&power[filter.first..])
                    .map(|(weight, level)| weight * level)
                    .sum();
                *value = (energy + LOG_FLOOR).max(LOG_FLOOR).ln();
            }
        }
        (features, silent)
    }
}

/// Removes from each mel bin its mean over the frames.
///
/// `fbank.mil` ends with `features = mel - reduce_mean(mel, frames)` over all
/// 998 frames, and `recenterOnSignal` (`OfflineEmbeddingExtractor.swift` line
/// 817, `SilenceAwareFbank.recenter`) then subtracts from that the mean over
/// the frames that are not digital silence. The two subtractions add up to one
/// subtraction of the mean over the non-silent frames, which is what is done
/// here, in one pass. A window with no non-silent frame keeps the plain mean
/// over all frames (the Swift leaves it untouched after the graph's mean), so
/// all its values are 0.
fn centre(features: &mut [f32], silent: &[bool]) {
    let voiced: usize = silent.iter().filter(|&&is_silent| !is_silent).count();
    let counted = |frame: usize| voiced == 0 || !silent[frame];
    let counted_frames: usize = if voiced == 0 { silent.len() } else { voiced };
    let mut sums: Vec<f64> = vec![0.0; MEL_BINS];
    for (frame, row) in features.as_chunks::<MEL_BINS>().0.iter().enumerate() {
        if counted(frame) {
            for (sum, &value) in sums.iter_mut().zip(row) {
                *sum += f64::from(value);
            }
        }
    }
    let means: Vec<f32> = sums
        .iter()
        .map(|&sum| (sum / counted_frames as f64) as f32)
        .collect();
    for row in features.as_chunks_mut::<MEL_BINS>().0 {
        for (value, &mean) in row.iter_mut().zip(&means) {
            *value -= mean;
        }
    }
}

/// `fbank.mil`, `_fbank_window`: the symmetric Hamming window of Kaldi's
/// fbank, `0.54 - 0.46 cos(2 pi i / (N - 1))`.
fn hamming_window() -> Vec<f32> {
    let denominator: f64 = (FRAME_LENGTH - 1) as f64;
    (0..FRAME_LENGTH)
        .map(|index| {
            let phase: f64 = 2.0 * std::f64::consts::PI * index as f64 / denominator;
            (0.54 - 0.46 * phase.cos()) as f32
        })
        .collect()
}

fn mel_scale(frequency: f64) -> f64 {
    1127.0 * (1.0 + frequency / 700.0).ln()
}

/// `fbank.mil`, `_fbank_mel_weight` (`[80, 257, 1]`): Kaldi's mel filter bank
/// over 20 Hz to the Nyquist frequency. Filters are triangles in the mel
/// domain with no area normalisation; the 257th spectrum bin (Nyquist) is in no
/// filter, as in Kaldi, which only knows the first 256.
fn mel_filters() -> Vec<MelBin> {
    let bin_width: f64 = SAMPLE_RATE as f64 / FFT_SIZE as f64;
    let mel_low: f64 = mel_scale(LOW_FREQ);
    let mel_high: f64 = mel_scale(SAMPLE_RATE as f64 / 2.0);
    let delta: f64 = (mel_high - mel_low) / (MEL_BINS + 1) as f64;
    let mel_of_bin: Vec<f64> = (0..FFT_SIZE / 2)
        .map(|index| mel_scale(bin_width * index as f64))
        .collect();

    (0..MEL_BINS)
        .map(|bin| {
            let left: f64 = mel_low + bin as f64 * delta;
            let center: f64 = mel_low + (bin + 1) as f64 * delta;
            let right: f64 = mel_low + (bin + 2) as f64 * delta;
            let weight_of = |mel: f64| -> f64 {
                if mel > left && mel < right {
                    if mel <= center {
                        (mel - left) / (center - left)
                    } else {
                        (right - mel) / (right - center)
                    }
                } else {
                    0.0
                }
            };
            let weights: Vec<f64> = mel_of_bin.iter().map(|&mel| weight_of(mel)).collect();
            let first: usize = weights.iter().position(|&w| w > 0.0).unwrap_or(0);
            let end: usize = weights
                .iter()
                .rposition(|&w| w > 0.0)
                .map_or(first, |i| i + 1);
            MelBin {
                first,
                weights: weights[first..end].iter().map(|&w| w as f32).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small seeded generator, so the tests need no `rand` crate.
    struct Lcg(u64);

    impl Lcg {
        fn next_unit(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        }
    }

    fn noise(len: usize, seed: u64, amplitude: f32) -> Vec<f32> {
        let mut generator = Lcg(seed);
        (0..len)
            .map(|_| generator.next_unit() * amplitude)
            .collect()
    }

    fn tone(len: usize, hz: f32, amplitude: f32) -> Vec<f32> {
        (0..len)
            .map(|index| {
                let phase =
                    2.0 * std::f64::consts::PI * f64::from(hz) * index as f64 / SAMPLE_RATE as f64;
                amplitude * phase.sin() as f32
            })
            .collect()
    }

    fn argmax(values: &[f32]) -> usize {
        let mut best: usize = 0;
        for (index, &value) in values.iter().enumerate() {
            if value > values[best] {
                best = index;
            }
        }
        best
    }

    fn row(features: &[f32], frame: usize) -> &[f32] {
        &features[frame * MEL_BINS..(frame + 1) * MEL_BINS]
    }

    #[test]
    fn frame_count_is_one_plus_samples_minus_frame_over_shift() {
        // fbank.mil `frames_1`: [?, 400, 998] for 160000 samples.
        assert_eq!(
            FBANK_FRAMES,
            1 + (WINDOW_SAMPLES - FRAME_LENGTH) / FRAME_SHIFT
        );
        assert_eq!(FBANK_FRAMES, 998);
        let fbank = Fbank::new();
        assert_eq!(
            fbank.window(&noise(WINDOW_SAMPLES, 1, 0.1)).len(),
            FBANK_FRAMES * MEL_BINS
        );
    }

    #[test]
    fn hamming_window_is_symmetric_and_starts_at_0_08() {
        let window = hamming_window();
        assert_eq!(window.len(), 400);
        // 0.54 - 0.46 cos(0) = 0.08, and cos(2 pi) = 1 at the last sample.
        assert!((window[0] - 0.08).abs() < 1e-6);
        assert!((window[399] - 0.08).abs() < 1e-6);
        for index in 0..400 {
            assert!((window[index] - window[399 - index]).abs() < 1e-6);
        }
        // The middle of an even-length symmetric window sits between samples
        // 199 and 200; both are just under the peak of 1.0.
        assert!(window[199] > 0.999 && window[199] <= 1.0);
    }

    #[test]
    fn mel_filters_match_hand_computed_weights() {
        // Worked out from the Kaldi formulas with a separate throwaway
        // program: mel(f) = 1127 ln(1 + f/700), mel(20) = 31.7486,
        // mel(8000) = 2840.0377, delta = (2840.0377 - 31.7486) / 81 = 34.6702.
        // FFT bin k is at 31.25 k Hz.
        // Bin 0 spans mel 31.7486 / 66.4188 / 101.0891 (left / center /
        // right). Only k=1 (mel 49.2218, rising side:
        // (49.2218 - 31.7486) / 34.6702 = 0.50398) and k=2 (mel 96.3835,
        // falling side: (101.0891 - 96.3835) / 34.6702 = 0.13572) fall inside.
        let filters = mel_filters();
        assert_eq!(filters.len(), 80);
        assert_eq!(filters[0].first, 1);
        assert_eq!(filters[0].weights.len(), 2);
        assert!((filters[0].weights[0] - 0.50398).abs() < 1e-4);
        assert!((filters[0].weights[1] - 0.13572).abs() < 1e-4);
        // Bin 79 spans mel 2770.6972 / 2805.3675 / 2840.0377: k=240 (7500 Hz,
        // mel 2773.3319) up to k=255 (7968.75 Hz). The center is between k=247
        // (0.93179) and k=248 (0.94777); the Nyquist bin 256 is in no filter.
        let last = &filters[79];
        assert_eq!(last.first, 240);
        assert_eq!(last.weights.len(), 16);
        assert!((last.weights[0] - 0.07599).abs() < 1e-4);
        assert!((last.weights[8] - 0.94777).abs() < 1e-4);
        assert!((last.weights[15] - 0.11697).abs() < 1e-4);
        assert!(filters.iter().all(|f| f.first + f.weights.len() <= 256));
    }

    #[test]
    fn tone_peaks_in_the_mel_bin_of_its_frequency() {
        // mel(1000 Hz) = 999.991, between the centers of bins 26 (967.845) and
        // 27 (1002.515): nearest is bin 27. mel(3000 Hz) = 1876.464, between
        // bin 52 (1869.271) and 53 (1903.941): nearest is bin 52.
        let fbank = Fbank::new();
        let first = fbank.log_mel(&tone_window(1000.0, 3000.0)).0;
        assert_eq!(argmax(row(&first, 100)), 27);
        assert_eq!(argmax(row(&first, 900)), 52);
    }

    fn tone_window(first_hz: f32, second_hz: f32) -> Vec<f32> {
        let half = WINDOW_SAMPLES / 2;
        let mut audio = tone(half, first_hz, 0.5);
        audio.extend(tone(WINDOW_SAMPLES - half, second_hz, 0.5));
        audio
    }

    #[test]
    fn centred_output_separates_the_two_halves() {
        // With the frame mean removed, the bin of the tone that fills the
        // first half is above the mean there and below it in the second half.
        let fbank = Fbank::new();
        let features = fbank.window(&tone_window(1000.0, 3000.0));
        assert_eq!(argmax(row(&features, 100)), 27);
        assert_eq!(argmax(row(&features, 900)), 52);
        assert!(row(&features, 100)[27] > 0.0 && row(&features, 900)[27] < 0.0);
    }

    #[test]
    fn mean_over_frames_of_every_bin_is_zero() {
        let fbank = Fbank::new();
        let features = fbank.window(&noise(WINDOW_SAMPLES, 7, 0.2));
        for bin in 0..MEL_BINS {
            let mean: f64 = features
                .as_chunks::<MEL_BINS>()
                .0
                .iter()
                .map(|frame| f64::from(frame[bin]))
                .sum::<f64>()
                / FBANK_FRAMES as f64;
            assert!(mean.abs() < 1e-4, "bin {bin} mean {mean}");
        }
    }

    #[test]
    fn digital_silence_gives_finite_zeros() {
        // Every frame is at the log floor, so the plain mean removes it all.
        let fbank = Fbank::new();
        let features = fbank.window(&vec![0.0; WINDOW_SAMPLES]);
        assert!(features.iter().all(|value| value.is_finite()));
        assert!(features.iter().all(|&value| value == 0.0));
        assert_eq!(features.len(), FBANK_FRAMES * MEL_BINS);
    }

    #[test]
    fn silent_frames_sit_at_the_log_floor_and_leave_the_mean() {
        // SilenceAwareFbank.swift lines 3-11: the mean ignores frames of exact
        // zeros. Half the window is noise, half digital silence.
        let fbank = Fbank::new();
        let mut audio = noise(WINDOW_SAMPLES / 2, 3, 0.2);
        audio.resize(WINDOW_SAMPLES, 0.0);
        let (raw, silent) = fbank.log_mel(&audio);
        // Frame f covers samples 160 f .. 160 f + 400. Frame 499 still reaches
        // sample 80239 inside the noise, frame 500 starts at 80000 in the zeros.
        assert!(!silent[499] && silent[500]);
        let floor = (f64::from(f32::EPSILON)).ln() as f32;
        assert!((raw[500 * MEL_BINS] - floor).abs() < 1e-5);

        let features = fbank.window(&audio);
        assert!(features.iter().all(|value| value.is_finite()));
        for bin in 0..MEL_BINS {
            let voiced_mean: f64 = (0..500)
                .map(|frame| f64::from(features[frame * MEL_BINS + bin]))
                .sum::<f64>()
                / 500.0;
            assert!(voiced_mean.abs() < 1e-4, "bin {bin}: {voiced_mean}");
            // The silent frames are far below the signal frames' mean.
            assert!(features[900 * MEL_BINS + bin] < -3.0);
        }
    }

    #[test]
    fn short_audio_is_zero_padded() {
        let fbank = Fbank::new();
        let short = noise(100_000, 11, 0.2);
        let mut padded = short.clone();
        padded.resize(WINDOW_SAMPLES, 0.0);
        assert_eq!(fbank.window(&short), fbank.window(&padded));
        assert_eq!(fbank.window(&[]), fbank.window(&vec![0.0; WINDOW_SAMPLES]));
    }

    #[test]
    fn long_audio_is_cut_to_one_window() {
        let fbank = Fbank::new();
        let long = noise(WINDOW_SAMPLES + 5000, 13, 0.2);
        assert_eq!(fbank.window(&long), fbank.window(&long[..WINDOW_SAMPLES]));
    }

    #[test]
    fn log_mel_of_a_frame_matches_a_direct_transform() {
        // A separate, naive evaluation of one frame (f64, O(n^2) DFT) against
        // the FFT path: mean removal, pre-emphasis with x[-1] = x[0], Hamming
        // window, power spectrum, mel filters, floor and log.
        let fbank = Fbank::new();
        let audio = noise(WINDOW_SAMPLES, 21, 0.3);
        let (got, _) = fbank.log_mel(&audio);
        let frame_index: usize = 37;
        let start = frame_index * FRAME_SHIFT;
        let scaled: Vec<f64> = audio[start..start + 400]
            .iter()
            .map(|&s| f64::from(s) * 32768.0)
            .collect();
        let mean: f64 = scaled.iter().sum::<f64>() / 400.0;
        let centred: Vec<f64> = scaled.iter().map(|s| s - mean).collect();
        let shaped: Vec<f64> = (0..400)
            .map(|i| {
                let previous = if i == 0 { centred[0] } else { centred[i - 1] };
                let w = 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / 399.0).cos();
                (centred[i] - 0.97 * previous) * w
            })
            .collect();
        let power: Vec<f64> = (0..257)
            .map(|k| {
                let mut re = 0.0;
                let mut im = 0.0;
                for (n, &x) in shaped.iter().enumerate() {
                    let angle = -2.0 * std::f64::consts::PI * (k * n) as f64 / 512.0;
                    re += x * angle.cos();
                    im += x * angle.sin();
                }
                re * re + im * im
            })
            .collect();
        let mel_low = mel_scale(20.0);
        let delta = (mel_scale(8000.0) - mel_low) / 81.0;
        for bin in 0..MEL_BINS {
            let left = mel_low + bin as f64 * delta;
            let center = left + delta;
            let right = center + delta;
            let mut energy = 0.0;
            for (k, &p) in power.iter().enumerate().take(256) {
                let mel = mel_scale(31.25 * k as f64);
                let weight = if mel > left && mel < right {
                    if mel <= center {
                        (mel - left) / (center - left)
                    } else {
                        (right - mel) / (right - center)
                    }
                } else {
                    0.0
                };
                energy += weight * p;
            }
            let want = (energy + 2f64.powi(-23)).max(2f64.powi(-23)).ln();
            let have = f64::from(got[frame_index * MEL_BINS + bin]);
            assert!(
                (want - have).abs() < 2e-3,
                "bin {bin}: want {want} have {have}"
            );
        }
    }

    /// `count` f32 values of the Core ML weight blob `fbank.bin` whose 64-byte
    /// header starts at `header_offset` (the `offset` of `fbank.mil`).
    fn blob_floats(blob: &[u8], header_offset: usize, count: usize) -> Vec<f32> {
        let start = header_offset + 64;
        blob[start..start + count * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect()
    }

    #[test]
    #[ignore = "needs fbank.bin in MURMUR_PLDA_DIR"]
    fn constants_match_the_core_ml_blob() {
        // `fbank.bin` sits next to the PLDA files in the folder MURMUR_PLDA_DIR
        // names. Offsets are those of `fbank.mil`.
        let dir = std::env::var("MURMUR_PLDA_DIR").expect("set MURMUR_PLDA_DIR");
        let blob = std::fs::read(format!("{dir}/fbank.bin")).expect("fbank.bin");

        let window = blob_floats(&blob, 1_135_168, 400);
        for (mine, theirs) in hamming_window().iter().zip(&window) {
            assert!((mine - theirs).abs() < 2e-5, "window {mine} vs {theirs}");
        }

        let mel = blob_floats(&blob, 64, 80 * 257);
        let filters = mel_filters();
        let mut worst: f32 = 0.0;
        for (bin, filter) in filters.iter().enumerate() {
            for index in 0..257usize {
                let mine: f32 = index
                    .checked_sub(filter.first)
                    .and_then(|i| filter.weights.get(i))
                    .copied()
                    .unwrap_or(0.0);
                worst = worst.max((mine - mel[bin * 257 + index]).abs());
            }
        }
        assert!(worst < 2e-5, "mel matrix differs by {worst}");

        // The frame kernel is the identity (it only cuts frames)...
        let kernel = blob_floats(&blob, 1_136_832, 400 * 400);
        for out in 0..400 {
            for tap in 0..400 {
                let want = if out == tap { 1.0 } else { 0.0 };
                assert_eq!(kernel[out * 400 + tap], want);
            }
        }
        // ...and the DFT weights are cos and sin of 2 pi k n / 512 up to sign
        // (the blob was written in f32 arithmetic: differences reach 2e-5).
        let real = blob_floats(&blob, 608_768, 257 * 512);
        let imag = blob_floats(&blob, 82_368, 257 * 512);
        for k in [0usize, 1, 17, 100, 256] {
            for n in [0usize, 1, 5, 399, 511] {
                let angle = 2.0 * std::f64::consts::PI * (k * n) as f64 / 512.0;
                assert!(
                    (f64::from(real[k * 512 + n]) - angle.cos()).abs() < 1e-4,
                    "real k={k} n={n}: {} vs {}",
                    real[k * 512 + n],
                    angle.cos()
                );
                assert!(
                    (f64::from(imag[k * 512 + n]).abs() - angle.sin().abs()).abs() < 1e-4,
                    "imag k={k} n={n}: {} vs {}",
                    imag[k * 512 + n],
                    angle.sin()
                );
            }
        }
    }

    #[test]
    fn nan_samples_do_not_poison_the_output() {
        let fbank = Fbank::new();
        let mut audio = noise(WINDOW_SAMPLES, 5, 0.2);
        audio[1000] = f32::NAN;
        let features = fbank.window(&audio);
        assert_eq!(features.len(), FBANK_FRAMES * MEL_BINS);
        assert!(features.iter().all(|value| value.is_finite()));
    }
}
