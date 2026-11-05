use super::*;
use crate::meetings::align::timing::time_chunk;
use crate::meetings::speakers;
use crate::meetings::tokens::{join, key, tokenize};

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

/// What the system track said and when (" | " between two sentences
/// Codex transcribed apart, 200 ms apart; after " + ", what it went on to
/// say 200 ms later, during the microphone sentence; after " / ", what it
/// said once that sentence was over), whether it has its own transcript
/// there, what the microphone wrote from 2.1 s (or from the time after
/// "@", in ms), the two levels, and the microphone sentences left. Every
/// reported case stays. Each row runs with the microphone's words timed
/// from what the on-device recogniser heard of them, and again with only
/// the sentence timed, unless the microphone pauses ("~"): an even spread
/// cannot place it.
#[allow(clippy::type_complexity)]
const CORPUS: &[(&str, u64, bool, &str, f32, f32, &[&str])] = &[
    (
        "on valide le budget jeudi prochain",
        2_000,
        true,
        "On valide le budget jeudi prochain.",
        -40.0,
        -30.0,
        &[],
    ),
    (
        "we'll send you the report before friday",
        2_000,
        true,
        "We'll send you the report before Friday.",
        -40.0,
        -30.0,
        &[],
    ),
    (
        "on valide le budget jeudi prochain",
        2_000,
        true,
        "Attends je ne suis pas d'accord.",
        -20.0,
        -30.0,
        &["Attends je ne suis pas d'accord."],
    ),
    (
        "oui merci",
        2_000,
        true,
        "Oui merci.",
        -40.0,
        -30.0,
        &["Oui merci."],
    ),
    // The user said it, and the remote side repeated it faintly.
    (
        "on valide le budget jeudi prochain",
        2_000,
        true,
        "On valide le budget jeudi prochain.",
        -20.0,
        -35.0,
        &["On valide le budget jeudi prochain."],
    ),
    (
        "on valide le budget jeudi prochain",
        30_000,
        true,
        "On valide le budget jeudi prochain.",
        -40.0,
        -30.0,
        &["On valide le budget jeudi prochain."],
    ),
    // Round 27: the system piece was not transcribed, so the
    // microphone's copy is the only one.
    (
        "oui ça me va",
        2_000,
        false,
        "Oui, ça me va.",
        -40.0,
        -30.0,
        &["Oui, ça me va."],
    ),
    // Round 18: a CJK sentence is one token, too short to judge.
    (
        "我们周四确定预算",
        2_000,
        true,
        "我们周四确定预算。",
        -40.0,
        -30.0,
        &["我们周四确定预算。"],
    ),
    // Round 32: the user's reply in the same sentence as the echo.
    (
        "on t'envoie le rapport avant vendredi",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, parfait.",
        -40.0,
        -30.0,
        &["d'accord, parfait."],
    ),
    (
        "we'll send you the report before friday",
        2_000,
        true,
        "We'll send you the report before Friday, okay, sounds good.",
        -40.0,
        -30.0,
        &["okay, sounds good."],
    ),
    (
        "on valide le budget jeudi prochain",
        2_000,
        true,
        "Oui, on valide le budget jeudi prochain.",
        -40.0,
        -30.0,
        &["Oui,"],
    ),
    (
        "we'll send you the report before friday",
        2_000,
        true,
        "Sure, we'll send you the report before Friday.",
        -40.0,
        -30.0,
        &["Sure,"],
    ),
    // The remote side goes on right after the reply, with a word of the
    // reply ("le", "I'll"): that word is not part of the echo.
    (
        "on t'envoie le rapport avant vendredi / super et le budget",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
        -40.0,
        -30.0,
        &["d'accord, je le relis."],
    ),
    (
        "we'll send you the report before friday / great and i'll share",
        2_000,
        true,
        "We'll send you the report before Friday, okay, I'll read it.",
        -40.0,
        -30.0,
        &["okay, I'll read it."],
    ),
    // Round 33: the remote side goes on during the reply (after " + ").
    (
        "on t'envoie le rapport avant vendredi + et le budget aussi",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
        -40.0,
        -30.0,
        &["d'accord, je le relis."],
    ),
    (
        "we'll send you the report before friday + and i'll send the slides",
        2_000,
        true,
        "We'll send you the report before Friday, okay, I'll read it.",
        -40.0,
        -30.0,
        &["okay, I'll read it."],
    ),
    (
        "on valide le budget jeudi prochain",
        2_000,
        true,
        "On valide le budget, oui, jeudi prochain.",
        -40.0,
        -30.0,
        &["oui,"],
    ),
    (
        "we'll send you the report before friday",
        2_000,
        true,
        "We'll send you the report, right, before Friday.",
        -40.0,
        -30.0,
        &["right,"],
    ),
    // Review 3: the remote side goes on during the reply with a word of
    // it ("je", "I'll"); the reply stays.
    (
        "on t'envoie le rapport avant vendredi + et je te mets en copie",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, je regarde.",
        -40.0,
        -30.0,
        &["d'accord, je regarde."],
    ),
    (
        "we'll send you the report before friday + and i'll copy you",
        2_000,
        true,
        "We'll send you the report before Friday, okay, I'll check.",
        -40.0,
        -30.0,
        &["okay, I'll check."],
    ),
    (
        "on t'envoie le rapport avant vendredi + et je te le renvoie",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
        -40.0,
        -30.0,
        &["d'accord, je le relis."],
    ),
    // Review 4: with headphones, the user repeats a time the remote side
    // said seconds earlier in the same long sentence; the repeat stays.
    (
        "on se voit jeudi à dix heures dans la grande salle avec toute l'équipe",
        2_000,
        true,
        "@6400 Jeudi, dix heures, ok.",
        -25.0,
        -20.0,
        &["Jeudi, dix heures, ok."],
    ),
    (
        "so we meet on thursday at ten in the big room with the whole team",
        2_000,
        true,
        "@6400 Thursday at ten, okay.",
        -25.0,
        -20.0,
        &["Thursday at ten, okay."],
    ),
    // Round 34: Codex ends a sentence of the system track where the
    // microphone's copy has a comma; the echo of both goes.
    (
        "on valide le budget | et le planning aussi",
        2_000,
        true,
        "On valide le budget, et le planning aussi.",
        -40.0,
        -30.0,
        &[],
    ),
    (
        "on valide le budget du projet | jeudi prochain",
        2_000,
        true,
        "On valide le budget du projet, jeudi prochain.",
        -40.0,
        -30.0,
        &[],
    ),
    (
        "we'll send the report | and the slides too",
        2_000,
        true,
        "We'll send the report, and the slides too.",
        -40.0,
        -30.0,
        &[],
    ),
    // The remote side's next sentence shares a word or two with the
    // reply; the reply stays.
    (
        "on t'envoie le rapport avant vendredi | et je te le renvoie",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
        -40.0,
        -30.0,
        &["d'accord, je le relis."],
    ),
    (
        "on t'envoie le rapport avant vendredi | et je te mets en copie",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, je regarde.",
        -40.0,
        -30.0,
        &["d'accord, je regarde."],
    ),
    (
        "on t'envoie le rapport avant vendredi | et le budget aussi",
        2_000,
        true,
        "On t'envoie le rapport avant vendredi, d'accord, je le relis.",
        -40.0,
        -30.0,
        &["d'accord, je le relis."],
    ),
    (
        "we'll send you the report before friday | and i'll copy you",
        2_000,
        true,
        "We'll send you the report before Friday, okay, I'll check.",
        -40.0,
        -30.0,
        &["okay, I'll check."],
    ),
    // Round 34: the user repeats the remote side's words in the same
    // sentence; the echo goes, the repeat stays.
    (
        "on se voit jeudi à dix heures",
        2_000,
        true,
        "On se voit jeudi à dix heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "we'll send you the report before friday",
        2_000,
        true,
        "We'll send you the report before Friday, before Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    // Round 35: the remote side pauses before the words the user
    // repeats; the echo still goes, not the repeat.
    (
        "on se voit jeudi ~1500 à dix heures",
        2_000,
        true,
        "On se voit jeudi ~1500 à dix heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "on se voit jeudi ~300 à dix heures",
        2_000,
        true,
        "On se voit jeudi ~300 à dix heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "we'll send you the report ~1500 before friday",
        2_000,
        true,
        "We'll send you the report ~1500 before Friday, before Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    (
        "we'll send you the report ~300 before friday",
        2_000,
        true,
        "We'll send you the report ~300 before Friday, before Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    // Round 36: the recogniser missed the faint echo of the words the
    // user repeats ("^"), so their time is only known to lie between the
    // words it heard around them, whatever the user said before.
    (
        "on se voit jeudi ~300 à dix heures",
        2_000,
        true,
        "On se voit jeudi ~300 ^à ^dix ^heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "on se voit jeudi ~1500 à dix heures",
        2_000,
        true,
        "On se voit jeudi ~1500 ^à ^dix ^heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "on se voit jeudi ~3000 à dix heures",
        2_000,
        true,
        "On se voit jeudi ~3000 ^à ^dix ^heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "on se voit jeudi ~1500 à dix heures",
        2_000,
        true,
        "@1300 Bonjour. ~400 On se voit jeudi ~1500 ^à ^dix ^heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["Bonjour.", "dix heures, parfait."],
    ),
    (
        "we'll send you the report ~300 before friday",
        2_000,
        true,
        "We'll send you the report ~300 ^before ^Friday, before Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    (
        "we'll send you the report ~1500 before friday",
        2_000,
        true,
        "We'll send you the report ~1500 ^before ^Friday, before Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    (
        "we'll send you the report ~3000 before friday",
        2_000,
        true,
        "We'll send you the report ~3000 ^before ^Friday, before Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    (
        "we'll send you the report ~1500 before friday",
        2_000,
        true,
        "@1300 Hello. ~400 We'll send you the report ~1500 ^before ^Friday, before Friday, great.",
        -40.0,
        -30.0,
        &["Hello.", "before Friday, great."],
    ),
    // Round 37: it missed the user's repeat too, so both copies may have
    // been said anywhere in the same stretch.
    (
        "on se voit jeudi ~1500 à dix heures",
        2_000,
        true,
        "@2000 On se voit jeudi ~1500 ^à ^dix ^heures, ^dix ^heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "we'll send you the report ~1500 before friday",
        2_000,
        true,
        "@2000 We'll send you the report ~1500 ^before ^Friday, ^before ^Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    // Round 38: the other way round, the remote side repeats the user
    // before going on, and it missed both copies; the user's words stay.
    (
        "dix heures c'est parfait on valide le budget",
        3_800,
        true,
        "@2000 ^Dix ^heures, ~1000 ^dix ^heures, c'est parfait, on valide le budget.",
        -40.0,
        -30.0,
        &["Dix heures,"],
    ),
    (
        "dix heures c'est parfait on valide le budget",
        4_600,
        true,
        "@2000 Jeudi à ^dix ^heures, ~1000 ^dix ^heures, c'est parfait, on valide le budget.",
        -40.0,
        -30.0,
        &["Jeudi à dix heures,"],
    ),
    (
        "at ten that works we'll send the report",
        3_800,
        true,
        "@2000 ^At ^ten, ~1000 ^at ^ten, that works, we'll send the report.",
        -40.0,
        -30.0,
        &["At ten,"],
    ),
    (
        "at ten that works we'll send the report",
        4_600,
        true,
        "@2000 So Friday ^at ^ten, ~1000 ^at ^ten, that works, we'll send the report.",
        -40.0,
        -30.0,
        &["So Friday at ten,"],
    ),
    // Round 39: the user repeats the remote side's words in a pause in
    // the remote's sentence, and it missed both copies; the echo is the
    // copy said when the system track said it, whichever runs on.
    (
        "on se voit à dix heures ~800 c'est bon pour toi",
        2_000,
        true,
        "@2000 On se voit à ^dix ^heures, ^dix ^heures, c'est bon pour toi ?",
        -40.0,
        -30.0,
        &["dix heures,"],
    ),
    (
        "we'll meet at ten ~800 does that work for you",
        2_000,
        true,
        "@2000 We'll meet ^at ^ten, ^at ^ten, does that work for you?",
        -40.0,
        -30.0,
        &["at ten,"],
    ),
    (
        "dix heures ~1000 c'est parfait on valide le budget",
        2_000,
        true,
        "@2000 ^Dix ^heures, ^dix ^heures, ~200 c'est parfait, on valide le budget.",
        -40.0,
        -30.0,
        &["dix heures,"],
    ),
    (
        "ten thirty ~1000 that works we'll send the report",
        2_000,
        true,
        "@2000 ^Ten ^thirty, ^ten ^thirty, ~200 that works, we'll send the report.",
        -40.0,
        -30.0,
        &["ten thirty,"],
    ),
    // A longer pause before the remote side's words than round 37's: an
    // even spread of the missed words would place the user's copy there.
    (
        "on se voit jeudi ~3000 à dix heures",
        2_000,
        true,
        "@2000 On se voit jeudi ~3000 ^à ^dix ^heures, ^dix ^heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "we'll send you the report ~3000 before friday",
        2_000,
        true,
        "@2000 We'll send you the report ~3000 ^before ^Friday, ^before ^Friday, great.",
        -40.0,
        -30.0,
        &["before Friday, great."],
    ),
    // Round 40: at the start of a chunk, it missed the remote side's
    // first words too, which were said slower than the pace the
    // sentence's start was estimated from.
    (
        "on se voit jeudi à dix heures",
        2_000,
        true,
        "^On ^se ^voit ^jeudi ^à ^dix ^heures, dix heures, parfait.",
        -40.0,
        -30.0,
        &["dix heures, parfait."],
    ),
    (
        "we'll meet on thursday at ten",
        2_000,
        true,
        "^We'll ^meet ^on ^Thursday ^at ^ten, at ten, sounds good.",
        -40.0,
        -30.0,
        &["at ten, sounds good."],
    ),
    // Round 41: at the start of a chunk, the user confirms a date or time
    // the remote side has just said, and it missed their first words,
    // long ones said faster than the slowest pace. The confirmation is
    // theirs, not an echo, even while the remote side goes on talking:
    // the remote side said those words before the user could have.
    (
        "on se voit mercredi prochain à quatorze heures | et on fera le point sur le budget",
        2_000,
        true,
        "@5400 ^Mercredi ^prochain ^quatorze ^heures, c'est noté. ~200 Je bloque le créneau.",
        -40.0,
        -30.0,
        &[
            "Mercredi prochain quatorze heures, c'est noté.",
            "Je bloque le créneau.",
        ],
    ),
    (
        "the deadline is september fifteenth | so please send the drafts before then",
        2_000,
        true,
        "@4600 ^September ^fifteenth, perfect. ~200 I'll let the team know.",
        -40.0,
        -30.0,
        &["September fifteenth, perfect.", "I'll let the team know."],
    ),
    (
        "on se voit mercredi prochain à quatorze heures et on fera le point sur le budget",
        2_000,
        true,
        "@5400 ^Mercredi ^prochain ^quatorze ^heures, c'est noté. ~200 Je bloque le créneau.",
        -40.0,
        -30.0,
        &[
            "Mercredi prochain quatorze heures, c'est noté.",
            "Je bloque le créneau.",
        ],
    ),
    (
        "the deadline is september fifteenth so please send the drafts before then",
        2_000,
        true,
        "@4600 ^September ^fifteenth, perfect. ~200 I'll let the team know.",
        -40.0,
        -30.0,
        &["September fifteenth, perfect.", "I'll let the team know."],
    ),
    (
        "on se voit mercredi prochain à quatorze heures | et on fera le point sur le budget",
        2_000,
        true,
        "@2000 ~3400 ^Mercredi ^prochain ^quatorze ^heures, c'est noté. ~200 Je bloque le créneau.",
        -40.0,
        -30.0,
        &[
            "Mercredi prochain quatorze heures, c'est noté.",
            "Je bloque le créneau.",
        ],
    ),
    (
        "the deadline is september fifteenth | so please send the drafts before then",
        2_000,
        true,
        "@2000 ~2600 ^September ^fifteenth, perfect. ~200 I'll let the team know.",
        -40.0,
        -30.0,
        &["September fifteenth, perfect.", "I'll let the team know."],
    ),
    (
        "on se voit mercredi prochain à quatorze heures et on fera le point sur le budget",
        2_000,
        true,
        "@2000 ~3400 ^Mercredi ^prochain ^quatorze ^heures, c'est noté. ~200 Je bloque le créneau.",
        -40.0,
        -30.0,
        &[
            "Mercredi prochain quatorze heures, c'est noté.",
            "Je bloque le créneau.",
        ],
    ),
    (
        "the deadline is september fifteenth so please send the drafts before then",
        2_000,
        true,
        "@2000 ~2600 ^September ^fifteenth, perfect. ~200 I'll let the team know.",
        -40.0,
        -30.0,
        &["September fifteenth, perfect.", "I'll let the team know."],
    ),
    // Round 42: a shorter confirmation, started 200 ms after the remote
    // side's last word: the earliest the user may have said a missed
    // word is the slowest pace already, with no slack on top.
    (
        "let's meet next tuesday | and we'll go over the budget",
        2_000,
        true,
        "@2000 ~1800 ^Next ^Tuesday, perfect. ~200 I'll book the room.",
        -40.0,
        -30.0,
        &["Next Tuesday, perfect.", "I'll book the room."],
    ),
    (
        "on se voit mercredi prochain | et on fera le point sur le budget",
        2_000,
        true,
        "@2000 ~2200 ^Mercredi ^prochain, parfait. ~200 Je note.",
        -40.0,
        -30.0,
        &["Mercredi prochain, parfait.", "Je note."],
    ),
    (
        "the deadline is september fifteenth | so please send the drafts before then",
        2_000,
        true,
        "@2000 ~2200 ^September ^fifteenth, perfect. ~200 I'll let the team know.",
        -40.0,
        -30.0,
        &["September fifteenth, perfect.", "I'll let the team know."],
    ),
    (
        "the deadline is september fifteenth so please send the drafts before then",
        2_000,
        true,
        "@2000 ~2200 ^September ^fifteenth, perfect. ~200 I'll let the team know.",
        -40.0,
        -30.0,
        &["September fifteenth, perfect.", "I'll let the team know."],
    ),
    // Accepted echo left in the transcript. A clause goes only when the
    // system track said every one of its words, whatever its neighbours,
    // so these rows, which expected the echo gone, now keep part of it
    // under "Me". A time, an amount or a name set off between echoed
    // clauses no longer goes with them, nor a last clause the system
    // track ends after the microphone sentence ("devant le bureau."):
    (
        "on se retrouve jeudi quinze heures trente devant le bureau",
        2_000,
        true,
        "On se retrouve jeudi, 15h30, devant le bureau.",
        -40.0,
        -30.0,
        &["15h30, devant le bureau."],
    ),
    (
        "we meet thursday three thirty by the office",
        2_000,
        true,
        "We meet Thursday, 3:30, by the office.",
        -40.0,
        -30.0,
        &["3:30,"],
    ),
    (
        "le budget quinze mille euros est validé",
        2_000,
        true,
        "Le budget, 15 000 euros, est validé.",
        -40.0,
        -30.0,
        &["15 000 euros,"],
    ),
    (
        "the budget fifteen thousand dollars is approved",
        2_000,
        true,
        "The budget, $15,000, is approved.",
        -40.0,
        -30.0,
        &["$15,000, is approved."],
    ),
    (
        "écoute jean baptiste on valide le budget",
        2_000,
        true,
        "Écoute, Jean-Baptiste, on valide le budget.",
        -40.0,
        -30.0,
        &["Écoute, Jean-Baptiste,"],
    ),
    // A word the recognisers spell apart, or that Codex misheard, keeps
    // its whole clause:
    (
        "on valide le budget jeudi prochain",
        2_000,
        true,
        "On valide le bidule jeudi prochain.",
        -40.0,
        -30.0,
        &["On valide le bidule jeudi prochain."],
    ),
    (
        "on se voit à quinze heures",
        2_000,
        true,
        "On se voit à 15h.",
        -40.0,
        -30.0,
        &["On se voit à 15h."],
    ),
    (
        "on en parle avec jean baptiste",
        2_000,
        true,
        "On en parle avec Jean-Baptiste.",
        -40.0,
        -30.0,
        &["On en parle avec Jean-Baptiste."],
    ),
    (
        "we ship in q three",
        2_000,
        true,
        "We ship in Q3.",
        -40.0,
        -30.0,
        &["We ship in Q3."],
    ),
    (
        "let's ask mary jane",
        2_000,
        true,
        "Let's ask Mary-Jane.",
        -40.0,
        -30.0,
        &["Let's ask Mary-Jane."],
    ),
    (
        "on se voit à quinze heures devant le bureau",
        2_000,
        true,
        "On se voit à 15h, oui, devant le bureau.",
        -40.0,
        -30.0,
        &["On se voit à 15h, oui,"],
    ),
    (
        "we meet at three thirty by the office",
        2_000,
        true,
        "We meet at 3:30, right, by the office.",
        -40.0,
        -30.0,
        &["We meet at 3:30, right,"],
    ),
    // Round 42: the round 40 echo, with the user pausing before their own
    // words. The echo's last words ended before the user may have said
    // them at the slowest pace without a pause, as round 42's quick
    // confirmations did, so the whole sentence stays:
    (
        "on se voit jeudi à dix heures",
        2_000,
        true,
        "^On ^se ^voit ^jeudi ^à ^dix ^heures, ~400 dix heures, parfait.",
        -40.0,
        -30.0,
        &["On se voit jeudi à dix heures, dix heures, parfait."],
    ),
    (
        "we'll meet on thursday at ten",
        2_000,
        true,
        "^We'll ^meet ^on ^Thursday ^at ^ten, ~400 at ten, sounds good.",
        -40.0,
        -30.0,
        &["We'll meet on Thursday at ten, at ten, sounds good."],
    ),
];

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
