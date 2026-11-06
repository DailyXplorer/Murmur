//! The speaker model of Windows against the answer of the Mac helper on the
//! same audio. The helper (`murmur-meetings-helper diarize`) runs FluidAudio's
//! Core ML pipeline, which only a Mac can run: a manual run of the
//! `meetings helper` workflow renders a fixture dialogue with `say`, keeps the
//! turns the helper found, and uploads both (`parity-dialogue` artifact). The
//! owner commits them next to this file's sibling fixtures as
//! `parity-dialogue.wav` and `parity-dialogue.turns.json`
//! (`OWNER_CHECKS.md` of the lane), and the ignored test below then compares.
//!
//! The comparison is plain Rust and tested here with invented turns. The
//! thresholds are the lane's acceptance numbers, not tuned to pass.

use std::collections::{BTreeMap, BTreeSet};

/// Change points of the two answers may differ by this much, in seconds.
const MAX_CHANGE_POINT_SHIFT: f64 = 1.0;
/// Share of the frames, after the best mapping of speakers, that must agree.
const MIN_AGREEMENT: f64 = 0.9;
/// The frames the agreement is counted on, in seconds.
const FRAME: f64 = 0.1;

/// A stretch of one speaker, in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub speaker: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub golden_speakers: usize,
    pub our_speakers: usize,
    /// The largest distance from a change of speaker in the golden answer to
    /// the nearest one in ours, or `None` when the golden answer has none.
    pub worst_change_point_shift: Option<f64>,
    /// Frames both answers give a speaker to, with the same speaker once the
    /// best mapping of ours onto the golden ones is applied.
    pub agreement: f64,
}

impl Comparison {
    pub fn is_close(&self) -> bool {
        self.golden_speakers == self.our_speakers
            && self
                .worst_change_point_shift
                .is_none_or(|shift| shift <= MAX_CHANGE_POINT_SHIFT)
            && self.agreement >= MIN_AGREEMENT
    }
}

/// The moments one speaker takes over from another, in time order.
fn change_points(turns: &[Turn]) -> Vec<f64> {
    let mut ordered: Vec<&Turn> = turns.iter().collect();
    ordered.sort_by(|a, b| a.start.total_cmp(&b.start));
    ordered
        .windows(2)
        .filter(|pair| pair[0].speaker != pair[1].speaker)
        .map(|pair| pair[1].start)
        .collect()
}

fn speaker_at(turns: &[Turn], at: f64) -> Option<&str> {
    turns
        .iter()
        .find(|turn| at >= turn.start && at < turn.end)
        .map(|turn| turn.speaker.as_str())
}

pub fn compare(golden: &[Turn], ours: &[Turn]) -> Comparison {
    let count = |turns: &[Turn]| {
        turns
            .iter()
            .map(|turn| turn.speaker.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    };
    let ours_changes = change_points(ours);
    let worst_change_point_shift = change_points(golden)
        .into_iter()
        .map(|golden_change| {
            ours_changes
                .iter()
                .map(|change| (change - golden_change).abs())
                .fold(f64::INFINITY, f64::min)
        })
        .fold(None, |worst: Option<f64>, shift| {
            Some(worst.map_or(shift, |worst| worst.max(shift)))
        });

    let end = golden
        .iter()
        .chain(ours)
        .map(|turn| turn.end)
        .fold(0.0, f64::max);
    let mut confusion: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut both = 0usize;
    let mut frame = 0usize;
    while (frame as f64 + 0.5) * FRAME < end {
        let at = (frame as f64 + 0.5) * FRAME;
        if let (Some(want), Some(got)) = (speaker_at(golden, at), speaker_at(ours, at)) {
            *confusion.entry((want, got)).or_default() += 1;
            both += 1;
        }
        frame += 1;
    }
    // The best one-to-one mapping, taking the biggest cells first.
    let mut cells: Vec<(&(&str, &str), &usize)> = confusion.iter().collect();
    cells.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let (mut used_golden, mut used_ours) = (BTreeSet::new(), BTreeSet::new());
    let mut agreed = 0usize;
    for ((want, got), frames) in cells {
        if used_golden.contains(want) || used_ours.contains(got) {
            continue;
        }
        used_golden.insert(*want);
        used_ours.insert(*got);
        agreed += frames;
    }
    Comparison {
        golden_speakers: count(golden),
        our_speakers: count(ours),
        worst_change_point_shift,
        agreement: if both == 0 {
            0.0
        } else {
            agreed as f64 / both as f64
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(speaker: &str, start: f64, end: f64) -> Turn {
        Turn {
            speaker: speaker.to_string(),
            start,
            end,
        }
    }

    fn dialogue() -> Vec<Turn> {
        vec![
            turn("S1", 0.0, 6.0),
            turn("S2", 6.5, 12.0),
            turn("S1", 12.5, 18.0),
            turn("S3", 18.5, 24.0),
        ]
    }

    #[test]
    fn the_same_turns_are_close() {
        let result = compare(&dialogue(), &dialogue());
        assert_eq!(result.golden_speakers, 3);
        assert_eq!(result.our_speakers, 3);
        assert_eq!(result.worst_change_point_shift, Some(0.0));
        assert!((result.agreement - 1.0).abs() < 1e-9);
        assert!(result.is_close());
    }

    #[test]
    fn other_names_for_the_same_voices_are_close() {
        let renamed: Vec<Turn> = dialogue()
            .into_iter()
            .map(|t| {
                let speaker = match t.speaker.as_str() {
                    "S1" => "S3",
                    "S2" => "S1",
                    _ => "S2",
                };
                turn(speaker, t.start, t.end)
            })
            .collect();
        let result = compare(&dialogue(), &renamed);
        assert!((result.agreement - 1.0).abs() < 1e-9);
        assert!(result.is_close());
    }

    #[test]
    fn a_change_point_half_a_second_off_is_close_and_two_seconds_off_is_not() {
        let shifted = |by: f64| -> Vec<Turn> {
            dialogue()
                .into_iter()
                .map(|t| {
                    if t.start > 10.0 {
                        turn(&t.speaker, t.start + by, t.end)
                    } else {
                        t
                    }
                })
                .collect()
        };
        let near = compare(&dialogue(), &shifted(0.5));
        assert!(near.worst_change_point_shift.unwrap() <= 0.5 + 1e-9);
        assert!(near.is_close());
        let far = compare(&dialogue(), &shifted(2.0));
        assert!(far.worst_change_point_shift.unwrap() > MAX_CHANGE_POINT_SHIFT);
        assert!(!far.is_close());
    }

    #[test]
    fn two_voices_taken_for_one_are_not_close() {
        let merged: Vec<Turn> = dialogue()
            .into_iter()
            .map(|t| {
                turn(
                    if t.speaker == "S3" { "S1" } else { &t.speaker },
                    t.start,
                    t.end,
                )
            })
            .collect();
        let result = compare(&dialogue(), &merged);
        assert_eq!(result.our_speakers, 2);
        assert!(!result.is_close());
    }

    #[test]
    fn a_voice_given_to_the_wrong_stretches_lowers_the_agreement() {
        let wrong = vec![
            turn("S1", 0.0, 6.0),
            turn("S2", 6.5, 12.0),
            turn("S2", 12.5, 18.0),
            turn("S3", 18.5, 24.0),
        ];
        let result = compare(&dialogue(), &wrong);
        assert!(result.agreement < 0.8, "{}", result.agreement);
        assert!(!result.is_close());
    }

    #[test]
    fn nothing_to_compare_is_never_close() {
        let result = compare(&dialogue(), &[]);
        assert_eq!(result.our_speakers, 0);
        assert_eq!(result.agreement, 0.0);
        assert!(!result.is_close());
        assert!(!compare(&[], &[]).is_close());
    }

    /// The Mac helper's answer, as `murmur-meetings-helper diarize` prints it.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[derive(serde::Deserialize)]
    struct Golden {
        segments: Vec<GoldenTurn>,
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[derive(serde::Deserialize)]
    struct GoldenTurn {
        spk: String,
        s: f64,
        e: f64,
    }

    /// Needs the models (`MURMUR_SMOKE_MODELS`) and the two files made on a
    /// Mac, which are not committed yet: see the module comment.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    #[ignore = "needs the golden turns made on a Mac"]
    fn the_turns_match_the_mac_helper_on_the_same_dialogue() {
        use crate::meetings::analysis_local::Diarizer;

        let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/src/meetings/fixtures/");
        let wav = format!("{fixtures}parity-dialogue.wav");
        let golden = format!("{fixtures}parity-dialogue.turns.json");
        let golden: Golden = serde_json::from_slice(
            &std::fs::read(&golden).expect("parity-dialogue.turns.json is not committed yet"),
        )
        .expect("the golden turns do not parse");
        let mut reader =
            hound::WavReader::open(&wav).expect("parity-dialogue.wav is not committed yet");
        let spec = reader.spec();
        assert_eq!((spec.channels, spec.sample_rate), (1, 16_000));
        let track: Vec<f32> = reader
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / f32::from(i16::MAX))
            .collect();

        let models = std::path::PathBuf::from(
            std::env::var("MURMUR_SMOKE_MODELS").expect("MURMUR_SMOKE_MODELS"),
        );
        let stop = || false;
        let mut diarizer = super::super::pyannote::load(&models, 2, &stop).expect("load");
        let spoken = diarizer.tell_apart(&track).expect("diarize");
        let ours: Vec<Turn> = spoken
            .iter()
            .map(|s| Turn {
                speaker: format!("S{}", s.speaker),
                start: s.start_ms as f64 / 1000.0,
                end: s.end_ms as f64 / 1000.0,
            })
            .collect();
        let golden: Vec<Turn> = golden
            .segments
            .into_iter()
            .map(|t| Turn {
                speaker: t.spk,
                start: t.s,
                end: t.e,
            })
            .collect();
        let result = compare(&golden, &ours);
        println!("{result:?}");
        assert!(result.is_close(), "{result:?}");
    }
}
