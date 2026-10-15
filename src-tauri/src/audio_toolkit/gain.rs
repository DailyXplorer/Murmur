//! Level normalisation for quiet speech before upload. Speech is told apart
//! from the background by 20 ms frame energy: only frames well above the
//! estimated noise floor and an absolute gate count as speech, the gain is
//! chosen from their level, and frames that are not speech keep unity gain
//! so silence and room noise are never raised into something a recogniser
//! could read words into. A soft limiter keeps amplified peaks below full
//! scale.

const FRAME: usize = 320;
const TARGET_SPEECH_DBFS: f32 = -20.0;
/// Below this a gain change is inaudible to the recogniser; leave audio as is.
const MIN_USEFUL_GAIN_DB: f32 = 3.0;
const MIN_SPEECH_FRAMES: usize = 10;
/// Frames below this hold no signal at all, not even a microphone's hiss.
const DIGITAL_SILENCE_DBFS: f32 = -100.0;
/// Speech must stand this far above the noise floor to be treated as speech.
const SPEECH_OVER_NOISE_DB: f32 = 9.0;
/// Amplified background must stay below this, whatever the speech level.
const MAX_NOISE_AFTER_GAIN_DBFS: f32 = -50.0;
const LIMITER_THRESHOLD: f32 = 0.85;
const LIMITER_CEILING: f32 = 0.98;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainProfile {
    /// Frames quieter than this are never speech.
    gate_dbfs: f32,
    max_gain_db: f32,
}

impl GainProfile {
    pub const fn new(whisper_mode: bool) -> Self {
        if whisper_mode {
            Self {
                gate_dbfs: -62.0,
                max_gain_db: 30.0,
            }
        } else {
            Self {
                gate_dbfs: -52.0,
                max_gain_db: 18.0,
            }
        }
    }
}

fn dbfs(rms: f32) -> f32 {
    20.0 * rms.max(1e-9).log10()
}

fn from_db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn frame_levels(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(FRAME)
        .map(|frame| {
            let energy: f32 = frame.iter().map(|sample| sample * sample).sum();
            dbfs((energy / frame.len() as f32).sqrt())
        })
        .collect()
}

fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    let index = ((sorted.len() - 1) as f32 * fraction).round() as usize;
    sorted[index]
}

/// The planned gain: which frames are speech and how much to raise them.
#[derive(Debug, PartialEq)]
struct GainPlan {
    gain: f32,
    speech: Vec<bool>,
}

fn plan(samples: &[f32], profile: GainProfile) -> Option<GainPlan> {
    let levels = frame_levels(samples);
    // Zero padding of short clips and mic warm-up are not room noise: left
    // in, they put the noise floor at -180 dBFS and disable both the
    // relative gate and the noise cap.
    let mut sorted: Vec<f32> = levels
        .iter()
        .copied()
        .filter(|level| *level > DIGITAL_SILENCE_DBFS)
        .collect();
    if sorted.len() < MIN_SPEECH_FRAMES {
        return None;
    }
    sorted.sort_by(f32::total_cmp);
    let noise_floor = percentile(&sorted, 0.1);
    let speech_threshold = profile.gate_dbfs.max(noise_floor + SPEECH_OVER_NOISE_DB);
    let speech: Vec<bool> = levels
        .iter()
        .map(|level| *level >= speech_threshold)
        .collect();
    let mut speech_levels: Vec<f32> = levels
        .iter()
        .zip(&speech)
        .filter_map(|(level, is_speech)| is_speech.then_some(*level))
        .collect();
    if speech_levels.len() < MIN_SPEECH_FRAMES {
        return None;
    }
    speech_levels.sort_by(f32::total_cmp);
    // A loud-ish percentile, so a few shouted syllables do not decide it.
    let speech_level = percentile(&speech_levels, 0.9);
    let gain_db = (TARGET_SPEECH_DBFS - speech_level)
        .min(profile.max_gain_db)
        .min(MAX_NOISE_AFTER_GAIN_DBFS - noise_floor);
    (gain_db >= MIN_USEFUL_GAIN_DB).then(|| GainPlan {
        gain: from_db(gain_db),
        speech,
    })
}

/// Soft knee above the threshold, asymptotic to the ceiling.
fn limit(sample: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude <= LIMITER_THRESHOLD {
        return sample;
    }
    let headroom = LIMITER_CEILING - LIMITER_THRESHOLD;
    let limited =
        LIMITER_THRESHOLD + headroom * ((magnitude - LIMITER_THRESHOLD) / headroom).tanh();
    limited.copysign(sample)
}

/// Returns louder samples when the recording holds quiet speech, `None`
/// when it should be uploaded unchanged.
pub fn normalize_quiet_speech(samples: &[f32], profile: GainProfile) -> Option<Vec<f32>> {
    let GainPlan { gain, speech } = plan(samples, profile)?;
    // Frame gains, widened by one frame each side so speech onsets and
    // tails are not cut, then ramped sample by sample to avoid clicks.
    let targets: Vec<f32> = (0..speech.len())
        .map(|index| {
            let near_speech = speech[index.saturating_sub(1)..(index + 2).min(speech.len())]
                .iter()
                .any(|is_speech| *is_speech);
            if near_speech {
                gain
            } else {
                1.0
            }
        })
        .collect();
    let mut output = Vec::with_capacity(samples.len());
    let mut current = targets[0];
    for (frame, target) in samples.chunks(FRAME).zip(&targets) {
        let step = (target - current) / frame.len() as f32;
        for sample in frame {
            current += step;
            output.push(limit(sample * current));
        }
        current = *target;
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = 16_000;

    /// Deterministic pseudo-random noise in [-amplitude, amplitude].
    fn noise(len: usize, amplitude: f32, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    fn tone(len: usize, amplitude: f32) -> Vec<f32> {
        (0..len)
            .map(|i| {
                (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / RATE as f32).sin() * amplitude
            })
            .collect()
    }

    /// Half a second of room noise, one second of "speech", half a second
    /// of room noise.
    fn quiet_dictation(speech_amplitude: f32, noise_amplitude: f32) -> Vec<f32> {
        let mut samples = noise(RATE / 2, noise_amplitude, 1);
        let speech = tone(RATE, speech_amplitude);
        let background = noise(RATE, noise_amplitude, 2);
        samples.extend(speech.iter().zip(&background).map(|(s, n)| s + n));
        samples.extend(noise(RATE / 2, noise_amplitude, 3));
        samples
    }

    fn rms_dbfs(samples: &[f32]) -> f32 {
        let energy: f32 = samples.iter().map(|s| s * s).sum();
        dbfs((energy / samples.len() as f32).sqrt())
    }

    #[test]
    fn decibel_conversions_agree() {
        assert!((dbfs(0.1) - -20.0).abs() < 1e-4);
        assert!((from_db(-20.0) - 0.1).abs() < 1e-6);
        assert!((from_db(6.0) - 1.9953).abs() < 1e-3);
    }

    #[test]
    fn digital_silence_is_never_amplified() {
        assert_eq!(
            normalize_quiet_speech(&vec![0.0; RATE * 2], GainProfile::new(true)),
            None
        );
    }

    #[test]
    fn steady_room_noise_alone_is_left_alone() {
        let room = noise(RATE * 2, 0.003, 9);
        assert_eq!(normalize_quiet_speech(&room, GainProfile::new(false)), None);
        assert_eq!(normalize_quiet_speech(&room, GainProfile::new(true)), None);
    }

    /// What the recorder hands over for a clip under a second: the samples
    /// followed by exact zeros up to 1.25 s.
    fn padded(mut samples: Vec<f32>) -> Vec<f32> {
        samples.resize(RATE * 5 / 4, 0.0);
        samples
    }

    #[test]
    fn padded_room_noise_is_left_alone() {
        // 0.8 s of fan noise at about -55 dBFS, an accidental tap.
        let room = noise(RATE * 4 / 5, 0.0031, 4);
        assert!((rms_dbfs(&room) - -55.0).abs() < 0.5);
        assert_eq!(
            normalize_quiet_speech(&padded(room.clone()), GainProfile::new(true)),
            None
        );
        let mut warm_up = vec![0.0; RATE / 2];
        warm_up.extend(&room);
        assert_eq!(
            normalize_quiet_speech(&warm_up, GainProfile::new(true)),
            None
        );
        let louder_room = noise(RATE * 4 / 5, 0.0055, 5);
        assert_eq!(
            normalize_quiet_speech(&padded(louder_room), GainProfile::new(false)),
            None
        );
    }

    #[test]
    fn padded_quiet_speech_is_still_raised() {
        let mut input = noise(RATE / 5, 0.0005, 6);
        let speech = tone(RATE / 2, 0.01);
        input.extend(
            speech
                .iter()
                .zip(noise(RATE / 2, 0.0005, 7))
                .map(|(s, n)| s + n),
        );
        assert!(normalize_quiet_speech(&padded(input), GainProfile::new(false)).is_some());
    }

    #[test]
    fn normal_speech_is_not_touched() {
        let loud = quiet_dictation(0.2, 0.001);
        assert_eq!(normalize_quiet_speech(&loud, GainProfile::new(false)), None);
    }

    #[test]
    fn quiet_speech_is_raised_and_the_pauses_are_not() {
        let input = quiet_dictation(0.01, 0.0005);
        let output = normalize_quiet_speech(&input, GainProfile::new(false)).unwrap();
        assert_eq!(output.len(), input.len());

        let speech = RATE / 2 + FRAME * 2..RATE / 2 + RATE - FRAME * 2;
        let gain_db = rms_dbfs(&output[speech.clone()]) - rms_dbfs(&input[speech]);
        assert!((gain_db - 18.0).abs() < 0.5, "speech gain {gain_db} dB");

        let pause = 0..RATE / 2 - FRAME * 2;
        let pause_gain_db = rms_dbfs(&output[pause.clone()]) - rms_dbfs(&input[pause]);
        assert!(pause_gain_db.abs() < 0.1, "pause gain {pause_gain_db} dB");
    }

    #[test]
    fn whisper_mode_allows_more_gain_than_normal_mode() {
        let input = quiet_dictation(0.005, 0.0001);
        let speech = RATE / 2 + FRAME * 2..RATE / 2 + RATE - FRAME * 2;
        let gain = |profile| {
            let output = normalize_quiet_speech(&input, profile).unwrap();
            rms_dbfs(&output[speech.clone()]) - rms_dbfs(&input[speech.clone()])
        };
        let normal = gain(GainProfile::new(false));
        let whisper = gain(GainProfile::new(true));
        assert!((normal - 18.0).abs() < 0.5, "normal {normal} dB");
        assert!(whisper > normal + 5.0, "whisper {whisper} dB");
    }

    #[test]
    fn gain_is_capped_so_the_noise_floor_stays_low() {
        // Speech only 12 dB above a fairly loud background: raising speech
        // to the target would lift the background above the noise ceiling.
        let input = quiet_dictation(0.008, 0.002);
        let plan = plan(&input, GainProfile::new(true)).unwrap();
        let mut levels = frame_levels(&input);
        levels.sort_by(f32::total_cmp);
        let noise_after = percentile(&levels, 0.1) + dbfs(plan.gain);
        assert!(
            noise_after <= MAX_NOISE_AFTER_GAIN_DBFS + 0.01,
            "{noise_after}"
        );
    }

    #[test]
    fn the_limiter_keeps_peaks_below_full_scale() {
        assert_eq!(limit(0.5), 0.5);
        assert_eq!(limit(-0.85), -0.85);
        assert!(limit(4.0) <= LIMITER_CEILING && limit(4.0) > 0.97);
        assert!(limit(-4.0) > -1.0);
        assert!(limit(0.9) > 0.85 && limit(0.9) < 0.9);

        let mut input = quiet_dictation(0.01, 0.0005);
        input[RATE] = 0.2;
        let output = normalize_quiet_speech(&input, GainProfile::new(true)).unwrap();
        assert!(output.iter().all(|sample| sample.abs() < 1.0));
    }
}
