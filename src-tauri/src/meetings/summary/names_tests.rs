//! The names the summary reads in the conversation (`NAMES`, `Named`).

use super::tests::{quick_policy, speaker, turn, Script};
use super::*;
use crate::OperationId;

fn unnamed(key: &str, number: u32) -> MeetingSpeaker {
    speaker(key, &format!("Speaker {number}"))
}

fn names(summary: &Summary) -> Vec<(&str, &str)> {
    summary
        .named
        .iter()
        .map(|named| (named.key.as_str(), named.name.as_str()))
        .collect()
}

#[test]
fn the_requests_that_read_the_transcript_ask_for_the_names_said_in_it() {
    for asked in [instructions(), section_instructions()] {
        assert!(asked.contains("`[[key]] = Name`"));
        assert!(asked.contains("[[me]] included"));
        assert!(asked.contains("never to the speaker who says it"));
        assert!(asked.contains("would be a guess"));
    }
    // The merge reads notes, where nobody speaks.
    assert!(!merge_instructions().contains("[[key]] = Name"));
}

#[test]
fn the_speakers_named_in_the_conversation_are_read_apart_from_the_notes() {
    let speakers = [speaker("me", "Me"), unnamed("s1", 2), unnamed("s2", 3)];
    // As a line, as a list item, in bold or in French quotation marks.
    let output = "# Budget\nPoint entre [[me]], [[s1]] et [[s2]].\n\n**Budget**\n- [[s1]] valide.\n\n[[s1]] = « Marie Dupont »\n- [[s2]] = **Paul**\n";
    let summary = parse(output, &speakers, Vec::new()).unwrap();
    assert_eq!(names(&summary), [("s1", "Marie Dupont"), ("s2", "Paul")]);
    assert_eq!(
        summary.body,
        "Point entre [[me]], [[s1]] et [[s2]].\n\n**Budget**\n- [[s1]] valide."
    );

    // A reply that names nobody is the notes it always was.
    let plain = parse("# Budget\nCorps.", &speakers, Vec::new()).unwrap();
    assert_eq!(plain.named, []);
    assert_eq!(plain.body, "Corps.");
}

#[test]
fn a_name_line_among_the_notes_is_not_left_in_them() {
    let summary = parse(
        "# T\nContexte.\n[[s1]] = Marie\n**Suite**\n- point",
        &[unnamed("s1", 2)],
        Vec::new(),
    )
    .unwrap();
    assert_eq!(summary.body, "Contexte.\n**Suite**\n- point");
    assert_eq!(names(&summary), [("s1", "Marie")]);
}

#[test]
fn only_a_name_of_another_speaker_of_the_meeting_is_taken() {
    let speakers = [speaker("me", "Me"), unnamed("s1", 2), unnamed("s2", 3)];
    let long = "x".repeat(crate::meetings::rename::MAX_SPEAKER_CHARS + 1);
    let refused = [
        ("[[me]] = Louis", "the user"),
        ("[[s9]] = Nobody", "a key the meeting has not"),
        ("[[_system]] = System", "the placeholder"),
        ("[[s1]] = speaker 3", "a name a speaker already shows"),
        ("[[s1]] = ", "no name"),
        (&format!("[[s1]] = {long}"), "a name too long for a rename"),
        ("[[s1]] = [[s2]]", "a marker"),
        (
            "[[s1]] = the one who leads the budget review today",
            "a sentence",
        ),
        ("[[s1]] = unknown: not said", "a remark"),
    ];
    for (line, why) in refused {
        let summary = parse(&format!("# T\nCorps.\n{line}\n"), &speakers, Vec::new()).unwrap();
        assert_eq!(summary.named, [], "{why}");
        assert_eq!(summary.body, "Corps.", "{why}");
    }

    // The first name of a speaker stands.
    let output = "# T\nCorps.\n[[s2]] = Paul\n[[s2]] = Pauline\n";
    let summary = parse(output, &speakers, Vec::new()).unwrap();
    assert_eq!(names(&summary), [("s2", "Paul")]);
}

#[test]
fn a_name_two_speakers_were_given_is_given_to_neither() {
    let speakers = [
        speaker("me", "Me"),
        unnamed("s1", 2),
        unnamed("s2", 3),
        unnamed("s3", 4),
        unnamed("s4", 5),
    ];
    // "Thanks Louis", said to the user, is the mistake this catches: the
    // name went to the speaker who said it.
    let output = "# T\nCorps.\n\
                  [[me]] = Louis Martin\n\
                  [[s1]] = louis\n\
                  [[s2]] = Sarah\n\
                  [[s3]] = Sarah\n\
                  [[s4]] = Marie Dupont\n";
    let summary = parse(output, &speakers, Vec::new()).unwrap();
    assert_eq!(names(&summary), [("s4", "Marie Dupont")]);

    // Two people who share a first name are two people.
    let output = "# T\nCorps.\n[[s1]] = Marie Dupont\n[[s2]] = Marie Curie\n";
    let summary = parse(output, &speakers, Vec::new()).unwrap();
    assert_eq!(
        names(&summary),
        [("s1", "Marie Dupont"), ("s2", "Marie Curie")]
    );
}

#[test]
fn a_later_name_of_a_speaker_still_tells_that_a_name_went_to_two() {
    let speakers = [speaker("me", "Me"), unnamed("s1", 2), unnamed("s2", 3)];
    // The parts of a long meeting: the first calls a speaker Liz, a later
    // one gives the same full name to both speakers.
    let said = vec![
        ("s1".to_string(), "Liz".to_string()),
        ("s1".to_string(), "Elizabeth Martin".to_string()),
        ("s2".to_string(), "Elizabeth Martin".to_string()),
    ];
    let summary = parse("# T\nCorps.", &speakers, said).unwrap();
    // The second name of the first speaker is not given, and it keeps the
    // other speaker from taking it.
    assert_eq!(names(&summary), [("s1", "Liz")]);
}

#[test]
fn the_title_never_takes_a_name_the_reply_only_proposes() {
    // The speaker may not take it: the user can rename them meanwhile.
    let output = "# Entretien avec [[s1]]\nCorps.\n[[s1]] = Marie";
    let summary = parse(output, &[unnamed("s1", 2)], Vec::new()).unwrap();
    assert_eq!(summary.title.as_deref(), Some("Entretien avec Speaker 2"));
    assert_eq!(names(&summary), [("s1", "Marie")]);
}

#[test]
fn a_name_a_speaker_shows_in_another_case_is_not_given() {
    let speakers = [speaker("s1", "Élodie"), unnamed("s2", 3)];
    let summary = parse("# T\nCorps.\n[[s2]] = élodie", &speakers, Vec::new()).unwrap();
    assert_eq!(summary.named, []);
}

#[test]
fn a_long_meeting_keeps_the_names_its_parts_gave_and_merges_notes_without_them() {
    let speakers = [speaker("me", "Me"), unnamed("s1", 2), unnamed("s2", 3)];
    let turns = [
        turn("s1", 2_000, "Bonjour, ici Marie."),
        turn("me", 5_000, "Bonjour Marie."),
        turn("s2", 3_725_000, "Paul à l'appareil, désolé du retard."),
    ];
    let script = Script::new(vec![
        Ok("# A\nPartie un.\n\n[[s1]] = Marie".into()),
        Ok("# B\nPartie deux.\n\n[[s2]] = Paul\n[[s1]] = Maria".into()),
        Ok("# Point\nTout.".into()),
    ]);
    // Two lines fit in one section, the third starts another.
    let end = tauri::async_runtime::block_on(summarize(
        &script,
        &quick_policy(),
        &turns,
        &speakers,
        100,
        Hints::default(),
        &ProcessingOperation::new(OperationId(1)),
    ));

    let SummaryEnd::Written(summary) = end else {
        panic!("no summary");
    };
    assert_eq!(summary.body, "Tout.");
    // The first part to name a speaker is believed.
    assert_eq!(names(&summary), [("s1", "Marie"), ("s2", "Paul")]);
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2]
        .1
        .contains("Notes of part 2 of 2:\n\n# B\nPartie deux."));
    assert!(!requests[2].1.contains(" = "));
}

/// Runs invented meetings through the model with the stored session, never
/// refreshing it, and writes the replies and the names read in them for a
/// person to judge.
#[test]
#[ignore]
fn live_names_write_samples() {
    use std::fmt::Write as _;

    // The lines of each meeting, the names a careful reader gives, and the
    // names nobody may be given: the user's, and people only talked about.
    type Meeting = (
        &'static str,
        &'static [(&'static str, &'static str)],
        &'static [(&'static str, &'static str)],
        &'static [&'static str],
    );
    let meetings: [Meeting; 5] = [
        (
            "fr, two introductions and a person who is absent",
            &[
                ("s1", "Bonjour à tous, c'est Marie Dupont de la comptabilité."),
                ("me", "Salut Marie, merci d'être là. On commence par le budget ?"),
                ("s2", "Oui. Et moi c'est Paul, je remplace Karim qui est en congé cette semaine."),
                ("s1", "Très bien. Le budget du trimestre est validé, il reste les frais de déplacement."),
                ("s2", "Je m'occupe des frais de déplacement d'ici vendredi."),
            ],
            &[("s1", "Marie Dupont"), ("s2", "Paul")],
            &["Karim"],
        ),
        (
            "en, a speaker addressed by name who answers, and one who says his name",
            &[
                ("me", "Sarah, can you walk us through the numbers?"),
                ("s1", "Sure. Revenue is up twelve percent and churn is flat."),
                ("s2", "Thanks Sarah. Tom here, by the way, sorry I joined late."),
                ("s1", "No problem. I will send the deck after the call."),
            ],
            &[("s1", "Sarah"), ("s2", "Tom")],
            &[],
        ),
        (
            "en, nobody is named",
            &[
                ("s1", "Shall we start with the release plan?"),
                ("me", "Yes. The build is green and the notes are written."),
                ("s2", "Then I will publish it this afternoon."),
            ],
            &[],
            &[],
        ),
        (
            "en, only the user and an absent person are named",
            &[
                ("s1", "Did Louis send the contract?"),
                ("me", "Yes, I sent it yesterday evening."),
                ("s2", "Great, thanks Louis. I will forward it to Anna in legal."),
            ],
            &[],
            &["Louis", "Anna"],
        ),
        (
            "fr, one imported track with three voices",
            &[
                ("s1", "Bienvenue dans ce nouvel épisode. Je reçois aujourd'hui Claire Martin, qui dirige un atelier de céramique."),
                ("s2", "Merci de l'invitation, je suis ravie d'être là."),
                ("s1", "Claire, comment avez-vous commencé ?"),
                ("s2", "Par hasard, pendant un stage d'été à Limoges."),
                ("s3", "Et pour les auditeurs qui nous rejoignent, je rappelle que l'émission est enregistrée en public."),
            ],
            &[("s2", "Claire Martin")],
            &[],
        ),
    ];

    let auth_path = codex_auth::auth_path().unwrap();
    let session = codex_auth::stored_session(&auth_path).unwrap();
    let codex_home = auth_path.parent().unwrap_or(&auth_path);
    let model = tauri::async_runtime::block_on(responses::default_model(
        &responses::models_url(responses::RESPONSES_URL),
        &session,
        codex_home,
    ));
    let instructions = instructions();
    let mut report = format!(
        "# Speaker name samples\n\n{} UTC, model `{model}`\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M")
    );
    let mut failures = Vec::new();
    for (what, lines, expected, forbidden) in meetings {
        let mut keys: Vec<&str> = lines.iter().map(|(key, _)| *key).collect();
        keys.sort_unstable();
        keys.dedup();
        let speakers: Vec<MeetingSpeaker> = keys
            .iter()
            .enumerate()
            .map(|(index, key)| match *key {
                "me" => speaker("me", "Me"),
                key => unnamed(key, index as u32 + 1),
            })
            .collect();
        let turns: Vec<MeetingTurn> = lines
            .iter()
            .enumerate()
            .map(|(index, (key, text))| turn(key, index as i64 * 9_000, text))
            .collect();
        let input = transcript_lines(&turns, &speakers).join("\n");
        let reply = tauri::async_runtime::block_on(responses::complete(
            responses::RESPONSES_URL,
            &session,
            &summary_request(&model, &instructions, &input),
            &ProcessingOperation::new(OperationId(14)),
        ));
        writeln!(report, "\n## {what}\n\n```\n{input}\n```\n").unwrap();
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                writeln!(report, "FAILED: {error:?}").unwrap();
                failures.push(format!("{what}: {error:?}"));
                continue;
            }
        };
        let summary = parse(&reply, &speakers, Vec::new()).unwrap();
        let given = names(&summary);
        writeln!(
            report,
            "Reply:\n\n```\n{}\n```\n\nNames: {given:?}",
            reply.trim()
        )
        .unwrap();
        if given != expected {
            failures.push(format!(
                "{what}: named {given:?}, a reader names {expected:?}"
            ));
        }
        for name in forbidden {
            if given.iter().any(|(_, given)| given.contains(name)) {
                failures.push(format!("{what}: {name} was given to a speaker"));
            }
        }
    }
    if !failures.is_empty() {
        writeln!(report, "\n## Differences\n\n- {}", failures.join("\n- ")).unwrap();
    }
    let samples = std::env::temp_dir().join("murmur-names-samples.md");
    std::fs::write(&samples, &report).unwrap();
    println!("{report}");
    assert!(
        failures.is_empty(),
        "see {}: {failures:?}",
        samples.display()
    );
}
