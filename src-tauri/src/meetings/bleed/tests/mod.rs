use super::*;
use crate::meetings::align::timing::time_chunk;
use crate::meetings::speakers;
use crate::meetings::tokens::{join, key, tokenize};

mod corpus;

use corpus::CORPUS;

fn mic(text: &str, start_ms: u64, end_ms: u64) -> Unit {
    Unit {
        track: Track::Mic,
        speaker: "me".to_string(),
        start_ms,
        end_ms,
        tokens: tokenize(text),
        low_confidence: false,
    }
}

fn heard(text: &str, start_ms: u64, step_ms: u64) -> Vec<TimedWord> {
    text.split(' ')
        .enumerate()
        .map(|(i, word)| TimedWord {
            key: key(word),
            start_ms: start_ms + i as u64 * step_ms,
            end_ms: start_ms + (i as u64 + 1) * step_ms,
        })
        .collect()
}

/// Whether `unit` echoes the system sentence of all the words in `system`.
fn is_echo(unit: &Unit, system: &[TimedWord], mic_levels: &[f32], levels: &[f32]) -> bool {
    let sentence = (system[0].start_ms, system[system.len() - 1].end_ms);
    !echoes(
        unit,
        0,
        &[sentence],
        system,
        WordTimes::Heard,
        mic_levels,
        levels,
    )
    .is_empty()
}

/// 20 s of levels: `db` everywhere.
fn flat(db: f32) -> Vec<f32> {
    vec![db; 1_000]
}

#[test]
fn a_sentence_the_system_track_said_at_the_same_time_is_an_echo() {
    let system = heard("on valide le budget jeudi prochain", 2_000, 400);
    let unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
    // The microphone hears the speakers 10 dB below the system track.
    assert!(is_echo(&unit, &system, &flat(-40.0), &flat(-30.0)));
    let mut said = mic("On valide le budget jeudi prochain.", 2_000, 4_400);
    said.track = Track::System;
    let mut units = vec![said, unit, mic("Moi je préfère lundi.", 6_000, 7_000)];
    assert_eq!(
        remove_echoes(
            &mut units,
            &[],
            &system,
            WordTimes::Heard,
            &flat(-40.0),
            &flat(-30.0)
        ),
        1
    );
    assert_eq!(units.len(), 2);
}

#[test]
fn an_echo_the_system_transcript_lacks_is_kept() {
    // A reply too short to send to Codex: its system chunk is silent,
    // so the microphone's copy is the only one in the transcript.
    let system = heard("oui ça me va", 2_000, 250);
    let unit = mic("Oui, ça me va.", 2_000, 3_000);
    assert!(is_echo(&unit, &system, &flat(-40.0), &flat(-30.0)));
    let mut units = vec![unit, Unit::placeholder(Track::System, 0, 600_000)];
    assert_eq!(
        remove_echoes(
            &mut units,
            &[],
            &system,
            WordTimes::Heard,
            &flat(-40.0),
            &flat(-30.0)
        ),
        0
    );
    assert_eq!(units.len(), 2);
}

#[test]
fn talking_over_someone_is_kept() {
    let system = heard("on valide le budget jeudi prochain", 2_000, 400);
    let unit = mic("Attends je ne suis pas d'accord.", 2_100, 4_500);
    assert!(!is_echo(&unit, &system, &flat(-20.0), &flat(-30.0)));
}

#[test]
fn short_sentences_are_kept() {
    let system = heard("oui merci", 2_000, 400);
    assert!(!is_echo(
        &mic("Oui merci.", 2_000, 2_800),
        &system,
        &flat(-40.0),
        &flat(-30.0)
    ));
}

#[test]
fn the_same_words_much_quieter_on_the_system_track_are_kept() {
    // The user said it, and the remote side repeated it faintly.
    let system = heard("on valide le budget jeudi prochain", 2_000, 400);
    let unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
    assert!(!is_echo(&unit, &system, &flat(-20.0), &flat(-35.0)));
}

#[test]
fn the_same_words_at_another_moment_are_kept() {
    let system = heard("on valide le budget jeudi prochain", 30_000, 400);
    let unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
    assert!(!is_echo(&unit, &system, &flat(-40.0), &flat(-30.0)));
}

#[test]
fn system_sentences_are_never_removed() {
    let system = heard("on valide le budget jeudi prochain", 2_000, 400);
    let mut unit = mic("On valide le budget jeudi prochain.", 2_100, 4_500);
    unit.track = Track::System;
    let mut units = vec![unit];
    assert_eq!(
        remove_echoes(
            &mut units,
            &[],
            &system,
            WordTimes::Heard,
            &flat(-40.0),
            &flat(-30.0)
        ),
        0
    );
}

/// The text of `said` without its pauses, when each of its tokens was
/// said from `start_ms` (400 ms a token, "~1500" a pause of 1.5 s), and
/// whether the on-device recogniser heard it ("^" before a word it
/// missed).
fn schedule(said: &str, start_ms: u64) -> (String, Vec<(u64, u64)>, Vec<bool>) {
    let mut words = Vec::new();
    let mut times = Vec::new();
    let mut heard = Vec::new();
    let mut at_ms = start_ms;
    for word in said.split(' ') {
        if let Some(pause_ms) = word.strip_prefix('~') {
            at_ms += pause_ms.parse::<u64>().unwrap();
            continue;
        }
        let missed = word.strip_prefix('^');
        let word = missed.unwrap_or(word);
        for _ in tokenize(word) {
            times.push((at_ms, at_ms + 400));
            heard.push(missed.is_none());
            at_ms += 400;
        }
        words.push(word);
    }
    (words.join(" "), times, heard)
}

#[test]
fn echo_corpus() {
    let mut failures = Vec::new();
    let end_ms = |start_ms: u64, times: &[(u64, u64)]| times.last().map_or(start_ms, |t| t.1);
    let said_at = |said: &str, start_ms: u64| {
        let (text, times, _) = schedule(said, start_ms);
        let words: Vec<TimedWord> = tokenize(&text)
            .into_iter()
            .zip(&times)
            .map(|(token, &(start, end))| TimedWord {
                key: token.key,
                start_ms: start,
                end_ms: end,
            })
            .collect();
        (text, words, end_ms(start_ms, &times))
    };
    for &(said, at_ms, transcribed, written, mic_db, system_db, kept) in CORPUS {
        let (mic_ms, written) = written
            .strip_prefix('@')
            .and_then(|rest| rest.split_once(' '))
            .map_or((2_100, written), |(ms, rest)| (ms.parse().unwrap(), rest));
        let (written, mic_times, mic_heard) = schedule(written, mic_ms);
        let mic_end_ms = end_ms(mic_ms, &mic_times);
        let (said, then) = said.split_once(" / ").unwrap_or((said, ""));
        let (said, meanwhile) = said.split_once(" + ").unwrap_or((said, ""));
        let mut system = Vec::new();
        let mut sentences = Vec::new();
        let mut next_ms = at_ms;
        for sentence in said.split(" | ") {
            let (text, words, sentence_end_ms) = said_at(sentence, next_ms);
            system.extend(words);
            let mut unit = mic(&text, next_ms, sentence_end_ms);
            unit.track = Track::System;
            sentences.push(unit);
            next_ms = sentence_end_ms + 200;
        }
        if !meanwhile.is_empty() {
            system.extend(said_at(meanwhile, next_ms).1);
        }
        if !then.is_empty() {
            system.extend(said_at(then, mic_end_ms + 200).1);
        }
        if !transcribed {
            sentences = vec![Unit::placeholder(Track::System, 0, 600_000)];
        }
        let paused = mic_times.windows(2).any(|pair| pair[0].1 < pair[1].0);
        let modes: &[bool] = if paused { &[true] } else { &[true, false] };
        for &measured in modes {
            let mut units = sentences.clone();
            if measured {
                let words: Vec<TimedWord> = tokenize(&written)
                    .into_iter()
                    .zip(&mic_times)
                    .zip(&mic_heard)
                    .filter(|(_, &heard)| heard)
                    .map(|((token, &(start_ms, end_ms)), _)| TimedWord {
                        key: token.key,
                        start_ms,
                        end_ms,
                    })
                    .collect();
                let timed = time_chunk(tokenize(&written), &words, mic_ms, mic_end_ms);
                let me = vec!["me".to_string(); timed.tokens.len()];
                units.extend(speakers::units(Track::Mic, timed.tokens, &me));
            } else {
                units.push(mic(&written, mic_ms, mic_end_ms));
            }
            remove_echoes(
                &mut units,
                &[mic_ms],
                &system,
                WordTimes::Heard,
                &flat(mic_db),
                &flat(system_db),
            );
            let left: Vec<String> = units
                .iter()
                .filter(|unit| unit.track == Track::Mic)
                .map(|unit| join(&unit.tokens))
                .collect();
            if left != kept {
                let timed = if measured { "heard" } else { "spread" };
                failures.push(format!(
                    "{written:?} ({timed}): got {left:?}, want {kept:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_reply_kept_from_an_echo_is_timed_from_the_echo() {
    let system = heard("on t'envoie le rapport avant vendredi", 2_000, 400);
    let mut said = mic("On t'envoie le rapport avant vendredi.", 2_000, 4_400);
    said.track = Track::System;
    let mut units = vec![
        said,
        mic("Oui, on t'envoie le rapport avant vendredi.", 1_500, 4_500),
        mic(
            "On t'envoie le rapport avant vendredi, d'accord, parfait.",
            2_000,
            5_400,
        ),
    ];
    remove_echoes(
        &mut units,
        &[],
        &system,
        WordTimes::Heard,
        &flat(-40.0),
        &flat(-30.0),
    );
    let mic_times: Vec<(u64, u64)> = units[1..]
        .iter()
        .map(|unit| (unit.start_ms, unit.end_ms))
        .collect();
    assert_eq!(mic_times, [(1_500, 2_000), (4_400, 5_400)]);
}

#[test]
fn a_reply_shares_no_word_with_what_the_remote_side_said_before_it() {
    // "then I'll call you" ends just before the user answers, and the
    // reply's "I'll" must not pull the echo back over it.
    let mut system = heard("then i'll call you", 200, 400);
    system.extend(heard("we'll send you the report before friday", 3_200, 400));
    let mut said = mic("We'll send you the report before Friday.", 3_200, 6_000);
    said.track = Track::System;
    let mut units = vec![
        said,
        mic(
            "Sure, I'll wait, we'll send you the report before Friday.",
            2_000,
            6_100,
        ),
    ];
    remove_echoes(
        &mut units,
        &[],
        &system,
        WordTimes::Heard,
        &flat(-40.0),
        &flat(-30.0),
    );
    let left: Vec<String> = units[1..].iter().map(|unit| join(&unit.tokens)).collect();
    assert_eq!(left, ["Sure, I'll wait,"]);
}

/// The words of the system sentence `text`, placed without the
/// on-device models: each over the whole sentence.
fn placed(text: &str, start_ms: u64, end_ms: u64) -> Vec<TimedWord> {
    tokenize(text)
        .into_iter()
        .filter(|token| !token.key.is_empty())
        .map(|token| TimedWord {
            key: token.key,
            start_ms,
            end_ms,
        })
        .collect()
}

/// The microphone sentences left once the echoes of the system sentence
/// `said`, placed at `[start_ms, end_ms)`, are removed from `units`.
fn left_with_placed_times(
    said: &str,
    (start_ms, end_ms): (u64, u64),
    mut units: Vec<Unit>,
    mic_db: f32,
) -> Vec<String> {
    let mut system = mic(said, start_ms, end_ms);
    system.track = Track::System;
    units.insert(0, system);
    remove_echoes(
        &mut units,
        &[0],
        &placed(said, start_ms, end_ms),
        WordTimes::Placed,
        &flat(mic_db),
        &flat(-30.0),
    );
    units
        .iter()
        .filter(|unit| unit.track == Track::Mic)
        .map(|unit| join(&unit.tokens))
        .collect()
}

/// Each track's sentences are spread over its own speech, so the echo
/// lands a second or two away from what the system track said.
#[test]
fn an_echo_placed_a_second_away_is_removed_without_heard_words() {
    let left = left_with_placed_times(
        "On valide le budget jeudi prochain.",
        (2_000, 4_400),
        vec![
            mic("Moi je préfère lundi.", 0, 1_500),
            mic("On valide le budget jeudi prochain.", 5_600, 8_000),
        ],
        -40.0,
    );
    assert_eq!(left, ["Moi je préfère lundi."]);
}

#[test]
fn talking_over_someone_is_kept_without_heard_words() {
    let left = left_with_placed_times(
        "On valide le budget jeudi prochain.",
        (2_000, 4_400),
        vec![mic("Attends, je ne suis pas d'accord.", 2_500, 4_500)],
        -20.0,
    );
    assert_eq!(left, ["Attends, je ne suis pas d'accord."]);
}

/// "On se rappelle." is what the user may well say back.
#[test]
fn a_three_word_echo_is_kept_without_heard_words() {
    let left = left_with_placed_times(
        "On se rappelle.",
        (2_000, 3_000),
        vec![mic("On se rappelle.", 2_000, 3_000)],
        -40.0,
    );
    assert_eq!(left, ["On se rappelle."]);
}

#[test]
fn the_mean_level_is_taken_as_power() {
    let mut levels = flat(-60.0);
    levels[50..100].iter_mut().for_each(|db| *db = -20.0);
    // Half the frames at -20 dB, half at -60: about -23 dB.
    let mean = mean_level(&levels, 1_000, 3_000).unwrap();
    assert!((mean - -23.0).abs() < 0.1, "{mean}");
    assert!(mean_level(&levels, 30_000, 31_000).is_none());
}
