//! The parts of Windows system-audio capture that need no Windows API: the
//! mono mixdown, the packet clock, the follow-the-default-output state
//! machine and the table from an endpoint's facts to a route. `wasapi.rs`
//! feeds them what the audio engine reports, so they run on synthetic data.

use std::time::{Duration, Instant};

use super::route::RouteFacts;

/// WASAPI stamps a packet with the performance counter in units of 100 ns.
const UNITS_PER_SECOND: i128 = 10_000_000;
const NANOS_PER_UNIT: u64 = 100;

/// Converts performance-counter ticks to the 100 ns units of a packet stamp.
pub(super) fn units_from_ticks(ticks: i64, frequency: i64) -> i64 {
    if frequency <= 0 {
        return 0;
    }
    (i128::from(ticks) * UNITS_PER_SECOND / i128::from(frequency)) as i64
}

/// Maps the performance counter onto `Instant`, which is that same counter
/// on Windows, so the microphone and the system track share one timeline.
/// Built once from a counter reading taken next to an `Instant::now()`.
#[derive(Debug, Clone, Copy)]
pub(super) struct ClockAnchor {
    instant: Instant,
    units: i64,
}

impl ClockAnchor {
    pub(super) fn new(instant: Instant, units: i64) -> Self {
        Self { instant, units }
    }

    fn instant_of(&self, units: u64) -> Instant {
        let delta = i128::from(units) - i128::from(self.units);
        let nanos = u64::try_from(delta.unsigned_abs())
            .unwrap_or(u64::MAX)
            .saturating_mul(NANOS_PER_UNIT);
        let shift = Duration::from_nanos(nanos);
        let moved = if delta >= 0 {
            self.instant.checked_add(shift)
        } else {
            self.instant.checked_sub(shift)
        };
        moved.unwrap_or(self.instant)
    }

    /// When the last sample of a packet of `frames` frames at `rate` was
    /// captured, which is what `Feeder::push_at` takes. `start_units` is the
    /// packet's stamp (the first frame); without one, or with one the engine
    /// flagged wrong, the packet ended `now`. Never later than `now`.
    pub(super) fn packet_end(
        &self,
        start_units: Option<u64>,
        frames: usize,
        rate: u32,
        now: Instant,
    ) -> Instant {
        let Some(start) = start_units.filter(|&units| units != 0) else {
            return now;
        };
        if rate == 0 {
            return now;
        }
        let length = Duration::from_secs_f64(frames as f64 / f64::from(rate));
        self.instant_of(start)
            .checked_add(length)
            .map_or(now, |end| end.min(now))
    }
}

/// Averages the channels of interleaved frames. A trailing partial frame is
/// dropped.
pub(super) fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

const FLOAT_TAG: u16 = 3;
const EXTENSIBLE_TAG: u16 = 0xFFFE;
const FLOAT_SUBFORMAT: u128 = 0x0000_0003_0000_0010_8000_00aa_0038_9b71;

/// Whether a shared-mode mix format holds 32-bit float samples, which is
/// what the audio engine mixes in. `subformat` is the GUID of an extensible
/// format.
pub(super) fn is_float32(tag: u16, bits: u16, subformat: Option<u128>) -> bool {
    bits == 32
        && match tag {
            FLOAT_TAG => true,
            EXTENSIBLE_TAG => subformat == Some(FLOAT_SUBFORMAT),
            _ => false,
        }
}

/// How often the default output is looked at, and so the longest a stream
/// keeps following a device that is no longer the default.
pub(super) const WATCH_INTERVAL: Duration = Duration::from_secs(1);
/// A rebuild that failed is tried again this often.
const RETRY_EVERY: Duration = Duration::from_secs(1);
/// A stream that stays down this long is reported as interrupted; a quick
/// switch between two headsets is not.
const INTERRUPTED_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Watch {
    Following(String),
    Down { since: Instant, retry_at: Instant },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Keep,
    Rebuild,
}

/// Which output the capture follows, and when to build it again.
#[derive(Debug, Clone)]
pub(super) struct DeviceWatch {
    state: Watch,
}

impl DeviceWatch {
    /// A stream was just built on `device`.
    pub(super) fn following(device: String) -> Self {
        Self {
            state: Watch::Following(device),
        }
    }

    /// Looks at the default output (`None` when there is none) and whether
    /// the running stream still delivers.
    pub(super) fn tick(&mut self, now: Instant, default: Option<&str>, stream_ok: bool) -> Step {
        match &self.state {
            Watch::Following(device) => {
                let moved = default.is_some_and(|id| id != device);
                if stream_ok && !moved {
                    return Step::Keep;
                }
                self.state = Watch::Down {
                    since: now,
                    retry_at: now + RETRY_EVERY,
                };
                Step::Rebuild
            }
            Watch::Down { since, retry_at } => {
                if default.is_none() || now < *retry_at {
                    return Step::Keep;
                }
                self.state = Watch::Down {
                    since: *since,
                    retry_at: now + RETRY_EVERY,
                };
                Step::Rebuild
            }
        }
    }

    /// A rebuild succeeded on `device`.
    pub(super) fn built(&mut self, device: String) {
        self.state = Watch::Following(device);
    }

    /// No stream for `INTERRUPTED_AFTER`.
    pub(super) fn interrupted(&self, now: Instant) -> bool {
        match &self.state {
            Watch::Following(_) => false,
            Watch::Down { since, .. } => now.saturating_duration_since(*since) >= INTERRUPTED_AFTER,
        }
    }
}

// The four-character codes `route::OutputRoute::of` reads. Endpoint facts
// are put in the same vocabulary so the table there stays the one place that
// decides.
const fn code(name: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*name)
}
const BUILT_IN: u32 = code(b"bltn");
const BLUETOOTH: u32 = code(b"blue");
const USB: u32 = code(b"usb ");
const HDMI: u32 = code(b"hdmi");
const INTERNAL_SPEAKER: u32 = code(b"ispk");
const HEADPHONES_JACK: u32 = code(b"hdpn");

// `EndpointFormFactor`.
const FORM_SPEAKERS: u32 = 1;
const FORM_LINE_LEVEL: u32 = 2;
const FORM_HEADPHONES: u32 = 3;
const FORM_HEADSET: u32 = 5;
const FORM_SPDIF: u32 = 8;
const FORM_DISPLAY: u32 = 9;

/// What Windows says about the default output, as `RouteFacts`. `enumerator`
/// is the bus that enumerated the device (`PKEY_Device_EnumeratorName`).
/// `has_input` stands for "worn on the head" here: a Bluetooth or USB device
/// whose form factor says headphones or headset. The form factor of a
/// Bluetooth speaker says speakers, so it never counts as headphones. `None`
/// when the form factor says nothing useful.
pub(super) fn route_facts(form_factor: Option<u32>, enumerator: &str) -> Option<RouteFacts> {
    let bus = enumerator.to_ascii_uppercase();
    let transport = if bus.starts_with("BTH") {
        BLUETOOTH
    } else if bus == "USB" {
        USB
    } else {
        BUILT_IN
    };
    let form_factor = form_factor?;
    let worn = matches!(form_factor, FORM_HEADPHONES | FORM_HEADSET);
    match form_factor {
        FORM_HEADPHONES | FORM_HEADSET | FORM_SPEAKERS | FORM_LINE_LEVEL => {
            if transport == BUILT_IN {
                Some(RouteFacts {
                    transport,
                    data_source: Some(if worn {
                        HEADPHONES_JACK
                    } else {
                        INTERNAL_SPEAKER
                    }),
                    has_input: false,
                })
            } else {
                Some(RouteFacts {
                    transport,
                    data_source: None,
                    has_input: worn,
                })
            }
        }
        FORM_SPDIF | FORM_DISPLAY => Some(RouteFacts {
            transport: HDMI,
            data_source: None,
            has_input: false,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::route::OutputRoute;
    use super::super::TrackWriter;
    use super::*;

    #[test]
    fn ticks_become_hundred_nanosecond_units() {
        assert_eq!(units_from_ticks(10_000_000, 10_000_000), 10_000_000);
        assert_eq!(units_from_ticks(3, 3), 10_000_000);
        // 19.2 MHz and 2.5 MHz counters.
        assert_eq!(units_from_ticks(19_200_000, 19_200_000), 10_000_000);
        assert_eq!(units_from_ticks(5_000_000, 2_500_000), 20_000_000);
        // A counter far into its uptime does not overflow.
        assert_eq!(units_from_ticks(i64::MAX / 2, 10_000_000), i64::MAX / 2);
        assert_eq!(units_from_ticks(5, 0), 0);
    }

    #[test]
    fn a_stereo_frame_is_averaged_to_one_sample() {
        let stereo = [1.0, 0.0, 0.0, 1.0, -1.0, -1.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![0.5, 0.5, -1.0]);
    }

    #[test]
    fn six_channels_are_averaged_and_a_partial_frame_is_dropped() {
        let mut frames = vec![0.5, 0.25, 0.0, -0.25, 0.5, 0.0];
        frames.extend([1.0, 0.0]);
        assert_eq!(downmix_to_mono(&frames, 6), vec![0.5 / 3.0]);
        assert!(downmix_to_mono(&[1.0, 2.0], 0).is_empty());
        assert_eq!(downmix_to_mono(&[0.25, 0.5], 1), vec![0.25, 0.5]);
    }

    fn anchor_at_one_second() -> (ClockAnchor, Instant) {
        let origin = Instant::now();
        // The counter read 1 s (10 000 000 units) when `origin` was taken.
        (ClockAnchor::new(origin, 10_000_000), origin)
    }

    #[test]
    fn a_packet_ends_where_its_stamp_and_length_say() {
        let (anchor, origin) = anchor_at_one_second();
        let now = origin + Duration::from_secs(60);
        // First frame 2.5 s on the counter, 480 frames at 48 kHz = 10 ms.
        let end = anchor.packet_end(Some(25_000_000), 480, 48_000, now);
        assert_eq!(end, origin + Duration::from_millis(1_510));
    }

    #[test]
    fn a_packet_stamped_before_the_anchor_lands_before_it() {
        let (anchor, origin) = anchor_at_one_second();
        let now = origin + Duration::from_secs(60);
        let end = anchor.packet_end(Some(5_000_000), 4_800, 48_000, now);
        // Started 0.5 s before the anchor, lasts 100 ms.
        assert_eq!(end, origin - Duration::from_millis(400));
    }

    #[test]
    fn a_packet_is_never_in_the_future() {
        let (anchor, origin) = anchor_at_one_second();
        let now = origin + Duration::from_millis(5);
        let end = anchor.packet_end(Some(100_000_000), 480, 48_000, now);
        assert_eq!(end, now);
    }

    #[test]
    fn a_packet_without_a_usable_stamp_ended_now() {
        let (anchor, origin) = anchor_at_one_second();
        let now = origin + Duration::from_secs(3);
        assert_eq!(anchor.packet_end(None, 480, 48_000, now), now);
        assert_eq!(anchor.packet_end(Some(0), 480, 48_000, now), now);
        assert_eq!(anchor.packet_end(Some(20_000_000), 480, 0, now), now);
    }

    fn read(path: &std::path::Path) -> Vec<f32> {
        hound::WavReader::open(path)
            .unwrap()
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / f32::from(i16::MAX))
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0, |peak: f32, sample| peak.max(sample.abs()))
    }

    /// WASAPI sends no packets while nothing plays. The packet after the
    /// pause carries its own stamp, and the track keeps the pause as silence.
    #[test]
    fn a_pause_between_packets_is_kept_as_silence_on_the_track() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system.wav");
        let origin = Instant::now();
        let writer = TrackWriter::create(&path, origin).unwrap();
        let feeder = writer.feeder();
        // The counter read 1 s when the meeting started.
        let anchor = ClockAnchor::new(origin, 10_000_000);
        let now = origin + Duration::from_secs(60);

        let tone = |seconds: f64| -> Vec<f32> {
            let frames = (48_000.0 * seconds) as usize;
            let stereo: Vec<f32> = (0..frames)
                .flat_map(|i| {
                    let value = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5;
                    [value, value]
                })
                .collect();
            downmix_to_mono(&stereo, 2)
        };

        // 0.0 to 0.5 s into the meeting, then nothing, then 2.0 to 2.5 s.
        for (start_units, seconds) in [(10_000_000u64, 0.5), (30_000_000, 0.5)] {
            let mono = tone(seconds);
            let end = anchor.packet_end(Some(start_units), mono.len(), 48_000, now);
            feeder.push_at(mono, 48_000, end);
        }
        let written = writer.finish(Duration::from_millis(2_500)).unwrap();

        let samples = read(&path);
        assert_eq!(written, 2_500 * 16);
        assert_eq!(samples.len(), 2_500 * 16);
        let ms = |from: usize, to: usize| &samples[from * 16..to * 16];
        assert!(peak(ms(50, 450)) > 0.3, "the first packet is on the track");
        assert!(peak(ms(600, 1_900)) < 0.001, "the pause is silence");
        assert!(peak(ms(2_050, 2_450)) > 0.3, "the second packet follows");
    }

    #[test]
    fn a_stream_stays_while_it_delivers_on_the_default_output() {
        let now = Instant::now();
        let mut watch = DeviceWatch::following("a".into());
        assert_eq!(watch.tick(now, Some("a"), true), Step::Keep);
        // No default output at the moment: the stream is not torn down yet.
        assert_eq!(watch.tick(now, None, true), Step::Keep);
        assert!(!watch.interrupted(now + Duration::from_secs(60)));
    }

    #[test]
    fn a_new_default_output_rebuilds_the_stream_once() {
        let now = Instant::now();
        let mut watch = DeviceWatch::following("a".into());
        assert_eq!(watch.tick(now, Some("b"), true), Step::Rebuild);
        // The rebuild is under way: no second one before the retry delay.
        assert_eq!(watch.tick(now, Some("b"), true), Step::Keep);
        watch.built("b".into());
        assert_eq!(watch.tick(now, Some("b"), true), Step::Keep);
    }

    #[test]
    fn the_default_output_is_looked_at_at_least_as_often_as_a_rebuild_is_retried() {
        assert!(WATCH_INTERVAL <= RETRY_EVERY);
    }

    #[test]
    fn a_stream_that_stopped_delivering_is_rebuilt_on_the_same_device() {
        let now = Instant::now();
        let mut watch = DeviceWatch::following("a".into());
        assert_eq!(watch.tick(now, Some("a"), false), Step::Rebuild);
    }

    #[test]
    fn a_failed_rebuild_is_retried_every_second_and_reported_after_five() {
        let start = Instant::now();
        let mut watch = DeviceWatch::following("a".into());
        assert_eq!(watch.tick(start, Some("b"), true), Step::Rebuild);
        let mut rebuilds = 0;
        for second in 1..=6 {
            let now = start + Duration::from_secs(second);
            if watch.tick(now, Some("b"), true) == Step::Rebuild {
                rebuilds += 1;
            }
            assert_eq!(
                watch.interrupted(now),
                second >= 5,
                "interrupted at {second} s"
            );
        }
        assert_eq!(rebuilds, 6);
    }

    #[test]
    fn nothing_is_rebuilt_while_there_is_no_output_device() {
        let start = Instant::now();
        let mut watch = DeviceWatch::following("a".into());
        assert_eq!(watch.tick(start, None, false), Step::Rebuild);
        let later = start + Duration::from_secs(10);
        assert_eq!(watch.tick(later, None, false), Step::Keep);
        assert!(watch.interrupted(later));
        // The headset comes back.
        assert_eq!(watch.tick(later, Some("a"), false), Step::Rebuild);
        watch.built("a".into());
        assert!(!watch.interrupted(later));
    }

    #[test]
    fn float_formats_are_told_from_the_rest() {
        assert!(is_float32(3, 32, None));
        assert!(is_float32(0xFFFE, 32, Some(FLOAT_SUBFORMAT)));
        assert!(!is_float32(0xFFFE, 32, Some(1)));
        assert!(!is_float32(0xFFFE, 32, None));
        assert!(!is_float32(1, 16, None));
        assert!(!is_float32(3, 64, None));
    }

    #[test]
    fn each_endpoint_is_told_apart_by_form_factor_and_bus() {
        use OutputRoute::*;
        let cases = [
            // Laptop speakers, a headphone jack, a USB headset, a Bluetooth
            // headset and a Bluetooth speaker.
            (Some(FORM_SPEAKERS), "HDAUDIO", Speakers),
            (Some(FORM_HEADPHONES), "HDAUDIO", Headphones),
            (Some(FORM_HEADSET), "USB", Headphones),
            (Some(FORM_HEADPHONES), "BTHENUM", Headphones),
            (Some(FORM_HEADSET), "BTHLEENUM", Headphones),
            (Some(FORM_SPEAKERS), "BTHENUM", Speakers),
            // USB speakers, a monitor, an optical output, a line out.
            (Some(FORM_SPEAKERS), "USB", Speakers),
            (Some(FORM_DISPLAY), "HDAUDIO", Speakers),
            (Some(FORM_SPDIF), "HDAUDIO", Speakers),
            (Some(FORM_LINE_LEVEL), "HDAUDIO", Speakers),
            (Some(FORM_SPEAKERS), "bthenum", Speakers),
            // Nothing readable, or a form factor that says nothing.
            (None, "HDAUDIO", Unknown),
            (Some(10), "HDAUDIO", Unknown),
            (Some(6), "BTHENUM", Unknown),
            (Some(0), "", Unknown),
        ];
        for (form_factor, bus, route) in cases {
            assert_eq!(
                OutputRoute::of(route_facts(form_factor, bus)),
                route,
                "{form_factor:?} on {bus}"
            );
        }
    }
}
