use super::*;

fn check(pass: impl Fn(&str) -> String, corpus: &[(&str, &str)]) {
    let failures: Vec<String> = corpus
        .iter()
        .filter_map(|(input, expected)| {
            let got = pass(input);
            (got != *expected).then(|| format!("{input:?}: got {got:?}, want {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn unchanged<'a>(texts: &[&'a str]) -> Vec<(&'a str, &'a str)> {
    texts.iter().map(|text| (*text, *text)).collect()
}

#[test]
fn fillers_corpus_en() {
    let pass = |text: &str| before_cleanup(text, PassLanguage::En);
    check(
        pass,
        &[
            ("Um, so I think we should go.", "So I think we should go."),
            ("I think, um, we should go.", "I think, we should go."),
            ("I um think so", "I think so"),
            ("that's it um.", "that's it."),
            ("that's it um", "that's it"),
            ("Uh, what time is it?", "What time is it?"),
            ("Hello. Um, how are you?", "Hello. How are you?"),
            ("um... I guess so", "I guess so"),
            ("Umm let me check", "Let me check"),
            ("and uhh the report", "and the report"),
            ("It's erm about ten", "It's about ten"),
            ("Er, I meant Tuesday.", "I meant Tuesday."),
            ("We can, uh, um, start now.", "We can, start now."),
            ("so uh… what now", "so what now"),
            ("Well uh I don't know uh maybe", "Well I don't know maybe"),
            ("Erm, the build failed.", "The build failed."),
            ("Um.", ""),
            ("Hello. Um. How are you?", "Hello. How are you?"),
        ],
    );
    check(
        pass,
        &unchanged(&[
            "Bring an umbrella tomorrow.",
            "Uhura was on the bridge.",
            "We went to the ER last night.",
            "UM is a short code here.",
            "The umpire called it.",
            "Uh-oh, that went badly.",
            "She bought a summer dress.",
            "Hum the tune for me.",
            "Euh is not an English word.",
            "The error was in the user module.",
            "Umami is the fifth taste.",
            "Herbs and other spices.",
            "Ernest arrived at noon.",
            "He said erase the board.",
            "This is UH, the airport code.",
            "Plain text with no hesitation at all.",
        ]),
    );
}

#[test]
fn fillers_corpus_fr() {
    let pass = |text: &str| before_cleanup(text, PassLanguage::Fr);
    check(
        pass,
        &[
            (
                "Euh, je pense qu'on peut y aller.",
                "Je pense qu'on peut y aller.",
            ),
            (
                "Je pense, euh, qu'on peut y aller.",
                "Je pense, qu'on peut y aller.",
            ),
            ("je euh pense", "je pense"),
            ("c'est tout euh.", "c'est tout."),
            ("c'est tout euh", "c'est tout"),
            ("Heu, on se voit demain ?", "On se voit demain ?"),
            ("Bonjour. Euh, ça va ?", "Bonjour. Ça va ?"),
            ("euh... je sais pas", "je sais pas"),
            ("Euhh attends", "Attends"),
            ("Hum, pas sûr.", "Pas sûr."),
            ("on a euh fini", "on a fini"),
            ("alors euh, hum, voilà", "alors voilà"),
            ("et heu le rapport", "et le rapport"),
            ("donc euh… on verra", "donc on verra"),
            ("Euh euh je reviens", "Je reviens"),
            ("le budget hum est validé", "le budget est validé"),
        ],
    );
    check(
        pass,
        &unchanged(&[
            "Le corps humain est fascinant.",
            "On part dans une heure.",
            "Il est humble et heureux.",
            "La humeur du jour est bonne.",
            "Un dia de sol, um dia de chuva.",
            "Er sagt, dass er kommt.",
            "Les heures sup sont payées.",
            "HUM est un sigle ici.",
            "Le reuh n'existe pas.",
            "Elle a fumé une cigarette.",
            "Le humus est riche.",
            "Ça sent l'humidité.",
            "Il a eu de la chance.",
            "Texte sans aucune hésitation.",
            "Un huit et un neuf.",
        ]),
    );
}

#[test]
fn breaks_corpus() {
    let en = |text: &str| before_cleanup(text, PassLanguage::En);
    let fr = |text: &str| before_cleanup(text, PassLanguage::Fr);
    check(
        en,
        &[
            (
                "first point new line second point",
                "first point\nSecond point",
            ),
            (
                "first point, new line, second point",
                "first point\nSecond point",
            ),
            ("Hello. New line. Thanks.", "Hello.\nThanks."),
            (
                "Dear team new paragraph the results",
                "Dear team\n\nThe results",
            ),
            (
                "Hi all, new paragraph, we shipped.",
                "Hi all\n\nWe shipped.",
            ),
            ("item one, new line, item two", "item one\nItem two"),
            ("thanks new line", "thanks\n"),
            ("Okay New Line next", "Okay\nNext"),
            ("a, new line new line b", "a\n\nB"),
            (
                "intro new paragraph body new line end",
                "intro\n\nBody\nEnd",
            ),
        ],
    );
    check(
        fr,
        &[
            (
                "premier point nouvelle ligne deuxième point",
                "premier point\nDeuxième point",
            ),
            (
                "Bonjour, nouveau paragraphe, voici le compte rendu.",
                "Bonjour\n\nVoici le compte rendu.",
            ),
            ("merci à la ligne Bonne journée", "merci\nBonne journée"),
            ("Il faut revenir à la ligne.", "Il faut revenir\n"),
            ("le titre, à la ligne, le texte", "le titre\nLe texte"),
            (
                "liste nouvelle ligne pain nouvelle ligne lait",
                "liste\nPain\nLait",
            ),
        ],
    );
    check(
        en,
        &unchanged(&[
            "We launched a new line of products.",
            "The new line manager starts Monday.",
            "Our new paragraph style is cleaner.",
            "This new line is faster.",
            "Every new line needs a test.",
            "Draw the new line here.",
            "Their new line of work is odd.",
            "A newline character ends it.",
            "Line up the new hires.",
            "The paragraph was new.",
        ]),
    );
    check(
        fr,
        &unchanged(&[
            "Il faut aller à la ligne de départ.",
            "Une nouvelle ligne de métro ouvre.",
            "La nouvelle ligne est ouverte.",
            "Un nouveau paragraphe de loi.",
            "Cette nouvelle ligne de bus passe ici.",
            "Il marche à la ligne droite près de la gare.",
            "Le point à la ligne reste au texte brut.",
        ]),
    );
}

#[test]
fn punctuation_corpus() {
    let en = |text: &str| raw_only(text, PassLanguage::En);
    let fr = |text: &str| raw_only(text, PassLanguage::Fr);
    check(
        en,
        &[
            (
                "hello comma how are you question mark",
                "hello, how are you?",
            ),
            ("we won exclamation mark", "we won!"),
            ("we won exclamation point great", "we won! Great"),
            ("that is all full stop thanks", "that is all. Thanks"),
            ("red comma green comma blue", "red, green, blue"),
            ("Is it done, question mark.", "Is it done?"),
            ("ready? Question mark", "ready?"),
            ("Hi team comma", "Hi team,"),
            ("first, comma, second", "first, second"),
            ("stop full stop", "stop."),
        ],
    );
    check(
        fr,
        &[
            (
                "bonjour virgule ça va point d'interrogation",
                "bonjour, ça va ?",
            ),
            ("c'est génial point d'exclamation", "c'est génial !"),
            (
                "merci point à la ligne bonne journée",
                "merci.\nBonne journée",
            ),
            ("oui virgule non", "oui, non"),
            ("tu viens point d’interrogation", "tu viens ?"),
            ("Bravo point d'exclamation on a gagné", "Bravo ! On a gagné"),
            ("pain virgule lait virgule œufs", "pain, lait, œufs"),
        ],
    );
    check(
        en,
        &unchanged(&[
            "Comma is a word I avoid.",
            "Put a comma after the name.",
            "The comma was misplaced.",
            "It ends with a question mark.",
            "That question mark is odd.",
            "We came to a full stop.",
            "Add an exclamation mark here.",
            "Every exclamation point counts.",
            "It was a trial period.",
            "Make your point clearly.",
            "The colon is a punctuation mark.",
            "Mark asked a question.",
            "A full stop ends the line.",
            "This comma splice is wrong.",
            "My question mark key is broken.",
            "There is no comma here.",
        ]),
    );
    check(
        fr,
        &unchanged(&[
            "Le prix est de trois virgule cinq euros.",
            "Il reste 2 virgule 5 litres.",
            "Mets une virgule ici.",
            "La virgule est mal placée.",
            "Il marque un point d'honneur.",
            "On fait le point demain.",
            "Ajoute deux points à la liste.",
            "Cette virgule est inutile.",
            "Il a mis un point d'interrogation à la fin.",
            "Le point d'exclamation est de trop.",
            "Virgule est un mot.",
            "Un point à la ligne suffit.",
            "Zéro virgule huit pour cent.",
            "Chaque virgule compte.",
            "Elle a ajouté des virgules.",
        ]),
    );
}

#[test]
fn detect_uses_setting_first() {
    let french = "je pense que la réunion est demain et que les notes sont prêtes";
    assert_eq!(detect("en", french), Some(PassLanguage::En));
    assert_eq!(detect("en-US", french), Some(PassLanguage::En));
    assert_eq!(detect("fr", "the cat and the dog"), Some(PassLanguage::Fr));
    assert_eq!(detect("de", french), None);
    assert_eq!(detect("zh-Hans", french), None);
}

#[test]
fn detect_auto_votes_on_stopwords() {
    assert_eq!(
        detect(
            "auto",
            "I think that the meeting is at three and you know it"
        ),
        Some(PassLanguage::En)
    );
    assert_eq!(
        detect("auto", "Je pense que la réunion est demain, c'est pas sûr"),
        Some(PassLanguage::Fr)
    );
    assert_eq!(detect("auto", "the end"), None, "fewer than 3 hits");
    assert_eq!(
        detect("auto", "the cat and the chat est la"),
        None,
        "not twice the other side"
    );
}

#[test]
fn detect_returns_none_for_portuguese_and_german() {
    for text in [
        "Um dia eu vou para o Brasil com a minha família e os meus amigos",
        "Er sagt, dass er morgen mit dem Zug nach Berlin kommt und bleibt",
        "Hoy es un día de sol y vamos a la playa con los niños",
    ] {
        assert_eq!(detect("auto", text), None, "{text}");
    }
}

#[test]
#[ignore = "timing, run with --release"]
fn text_passes_bench() {
    let sentence = "Um, so I think, uh, we should ship it new line and then comma \
        maybe wait question mark the team said that it is fine to go ahead. ";
    let words = sentence.split_whitespace().count();
    let text = sentence.repeat(2_000 / words + 1);
    let runs = 200;
    let started = std::time::Instant::now();
    for _ in 0..runs {
        let cleaned = before_cleanup(std::hint::black_box(&text), PassLanguage::En);
        std::hint::black_box(raw_only(&cleaned, PassLanguage::En));
    }
    let per_run = started.elapsed() / runs;
    println!(
        "text passes over {} words: {} µs per dictation",
        text.split_whitespace().count(),
        per_run.as_micros()
    );
}
