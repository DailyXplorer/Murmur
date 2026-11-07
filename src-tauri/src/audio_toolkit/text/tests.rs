use super::*;

/// The default custom-word threshold (`settings.rs`).
const THRESHOLD: f64 = 0.18;

const PRODUCTS: &[&str] = &["iPhone", "ChatGPT", "OpenAI", "ChargeBee"];
const ENDING_WORDS: &[&str] = &["ChatGPT", "iPhone", "Figma", "Dan"];
const ELISION_WORDS: &[&str] = &[
    "iPhone",
    "Anthropic",
    "Excel",
    "D'Angelo",
    "Apple",
    "Emile",
    "Laura",
    "Dune",
];

/// Every dictated sentence the custom-word pass has been checked on:
/// custom words, what Codex wrote, what is pasted. Add a row for each
/// new case; never edit an old one to make a change pass.
const CORPUS: &[(&[&str], &str, &str)] = &[
    (&[], "hello world", "hello world"),
    (&["Hello", "World"], "hello world", "Hello World"),
    (&["hello", "Murmur"], "helo from Murmur", "helo from Murmur"),
    // A lone capital letter is not shouting.
    (&["iPhone", "Xcode"], "I phone died", "iPhone died"),
    (&["iPhone", "Xcode"], "open X code now", "open Xcode now"),
    (
        &["iPhone", "Xcode"],
        "Hi. X code crashed.",
        "Hi. Xcode crashed.",
    ),
    // A fuzzy span starting one word early must not beat the exact span
    // after it.
    (PRODUCTS, "my I phone died", "my iPhone died"),
    (PRODUCTS, "the chat GPT app", "the ChatGPT app"),
    (PRODUCTS, "open AI is", "OpenAI is"),
    (PRODUCTS, "charge B is great", "ChargeBee is great"),
    // A trailing ordinary word adds nothing to the match.
    (
        &["ChargeBee"],
        "we use charge it daily",
        "we use charge it daily",
    ),
    (&["ChatGPT"], "the chat app works", "the chat app works"),
    // Adjacent custom words each keep their own span.
    (PRODUCTS, "open AI chat GPT", "OpenAI ChatGPT"),
    (
        PRODUCTS,
        "my I phone, chat GPT and charge B.",
        "my iPhone, ChatGPT and ChargeBee.",
    ),
    (PRODUCTS, "mon I phone est mort", "mon iPhone est mort"),
    (
        PRODUCTS,
        "j'utilise chat GPT tous les jours",
        "j'utilise ChatGPT tous les jours",
    ),
    (
        PRODUCTS,
        "on a signé avec charge B hier",
        "on a signé avec ChargeBee hier",
    ),
    (
        PRODUCTS,
        "le modèle open AI est rapide",
        "le modèle OpenAI est rapide",
    ),
    (PRODUCTS, "we met at noon", "we met at noon"),
    (PRODUCTS, "je mange une pomme", "je mange une pomme"),
    (PRODUCTS, "my iPhone died", "my iPhone died"),
    // Multi-word spans.
    (
        &["ChargeBee"],
        "il cui nome è Charge B, che permette",
        "il cui nome è ChargeBee, che permette",
    ),
    (
        &["ChatGPT"],
        "use Chat G P T for this",
        "use ChatGPT for this",
    ),
    (&["OpenAI", "GPT"], "Open AI GPT model", "OpenAI GPT model"),
    (&["ChargeBee"], "CHARGE B is great", "CHARGEBEE is great"),
    (&["MacBook Pro"], "using Mac Book Pro", "using MacBook Pro"),
    (&["GPT-4"], "use GPT4 for this", "use GPT-4 for this"),
    (
        &["R&D"],
        "send it to RD for review",
        "send it to R&D for review",
    ),
    (
        &["R&D"],
        "send it to R and D for review",
        "send it to R&D for review",
    ),
    (
        &["R&D"],
        "send it to R&D for review",
        "send it to R&D for review",
    ),
    (&["Murmur"], "「Murmurr。」", "「Murmur。」"),
    // Possessive and plural endings stay on the custom word.
    (ENDING_WORDS, "ChatGPT's answer", "ChatGPT's answer"),
    (ENDING_WORDS, "ChatGPT’s answer", "ChatGPT’s answer"),
    (ENDING_WORDS, "both iPhones died", "both iPhones died"),
    (ENDING_WORDS, "two Figmas.", "two Figmas."),
    (ENDING_WORDS, "ask chat gpt's view", "ask ChatGPT's view"),
    (ENDING_WORDS, "my I phones", "my iPhones"),
    (
        ENDING_WORDS,
        "je vais dans le bureau",
        "je vais dans le bureau",
    ),
    // A short name never rewrites an everyday word.
    (&["Anne"], "I ate an apple", "I ate an apple"),
    (&["Anne"], "il y a un an", "il y a un an"),
    (&["Marc"], "ça marche bien", "ça marche bien"),
    (&["Marie"], "mon mari arrive", "mon mari arrive"),
    (&["Jean"], "j'en ai marre", "j'en ai marre"),
    (&["Paul"], "un pull rouge", "un pull rouge"),
    // The name itself, in any case, still takes the user's spelling.
    (&["Anne"], "ask anne first", "ask Anne first"),
    (&["Marc"], "MARC est là", "MARC est là"),
    // A short name with one letter wrong is corrected mid-sentence.
    (&["Zach"], "ask Zack first", "ask Zach first"),
    (&["Luka"], "je vois Luca demain", "je vois Luka demain"),
    (&["Sarah"], "call Sara today", "call Sarah today"),
    (&["Zach"], "ask Zack's mother", "ask Zach's mother"),
    (
        &["Zach"],
        "j'ai dit «Zack» deux fois",
        "j'ai dit «Zach» deux fois",
    ),
    // A title's full stop ends no sentence.
    (&["Zach"], "ask Dr. Zack first", "ask Dr. Zach first"),
    (&["Luka"], "j'ai vu M. Luca hier", "j'ai vu M. Luka hier"),
    (&["Zach"], "ask Ms. Zack first", "ask Ms. Zach first"),
    (&["Zach"], "ask Prof. Zack first", "ask Prof. Zach first"),
    (
        &["Luka"],
        "j'ai vu le Pr. Luca hier",
        "j'ai vu le Pr. Luka hier",
    ),
    // An acronym or a unit before a full stop is no title.
    (
        &["Nina"],
        "I opened a PR. Nine people came.",
        "I opened a PR. Nine people came.",
    ),
    (
        &["Nina"],
        "We ran 3 m. Nine people came.",
        "We ran 3 m. Nine people came.",
    ),
    // A short everyday word that sounds like a name is kept.
    (&["Lou"], "je l'ai lu hier", "je l'ai lu hier"),
    (&["Mona"], "c'est mon idée", "c'est mon idée"),
    (&["Mona"], "Mon idée est simple", "Mon idée est simple"),
    (&["Moon"], "mon téléphone", "mon téléphone"),
    (&["May"], "le 3 mai", "le 3 mai"),
    // Not an everyday word on the list: only the length keeps it.
    (&["Liz"], "je lis un livre", "je lis un livre"),
    // A capital that does not mark a name: a proper noun, a contraction,
    // an acronym, or the first word of a sentence.
    (&["Max"], "my Mac is slow", "my Mac is slow"),
    (&["Max"], "Mon Mac plante", "Mon Mac plante"),
    (&["Ivy"], "I've sent it", "I've sent it"),
    (&["Ilya"], "I'll call you", "I'll call you"),
    (&["Leon"], "Je vais à Lyon", "Je vais à Lyon"),
    (&["Mona"], "I told Mom", "I told Mom"),
    (&["Nina"], "Nine people came.", "Nine people came."),
    (
        &["Nina"],
        "Done. Nine people came.",
        "Done. Nine people came.",
    ),
    (&["Sarah"], "Sara, can you call me", "Sara, can you call me"),
    (&["Sarah"], "Présents : Sara", "Présents : Sara"),
    // A closing mark written apart, after the sentence end.
    (
        &["Alex"],
        "Il a crié « on a gagné ! » Allez, on rentre.",
        "Il a crié « on a gagné ! » Allez, on rentre.",
    ),
    (
        &["Alex"],
        "Il a crié \" on a gagné ! \" Allez, on rentre.",
        "Il a crié \" on a gagné ! \" Allez, on rentre.",
    ),
    (
        &["Alex"],
        "Il a crié (on a gagné ! ) Allez, on rentre.",
        "Il a crié (on a gagné ! ) Allez, on rentre.",
    ),
    (&["Pam"], "the PM said", "the PM said"),
    (&["Jess"], "le code JS", "le code JS"),
    (&["Avi"], "the API returns", "the API returns"),
    (
        &["Java"],
        "J'avoue, c'est pratique",
        "J'avoue, c'est pratique",
    ),
    (&["Benny"], "Ben, je sais pas", "Ben, je sais pas"),
    (&["Benny"], "ouais Ben je sais pas", "ouais Ben je sais pas"),
    // A first name that is also an everyday word is kept.
    (&["Clara"], "une réponse claire", "une réponse claire"),
    (&["Ross"], "un pull rose", "un pull rose"),
    (&["Rosa"], "la robe rose", "la robe rose"),
    (&["Jeanne"], "un jean bleu", "un jean bleu"),
    (&["Pierce"], "une pierre", "une pierre"),
    (
        &["Simon"],
        "Sinon, on verra demain",
        "Sinon, on verra demain",
    ),
    (
        &["Simon"],
        "appelle-moi, sinon je t'écris",
        "appelle-moi, sinon je t'écris",
    ),
    // A custom spelling overrides a common first name.
    (&["Lukas"], "Lucas a appelé", "Lukas a appelé"),
    (&["Nicholas"], "merci Nicolas", "merci Nicholas"),
    (&["Sofie"], "Sophie arrive", "Sofie arrive"),
    (&["Tomas"], "Thomas arrive", "Tomas arrive"),
    (&["Philipe"], "Philippe arrive", "Philipe arrive"),
    (&["O'Brien"], "call o'brien today", "call O'Brien today"),
    // An accent the transcriber dropped or added is put back as the
    // user wrote it.
    (&["Zoé"], "appelle Zoe demain", "appelle Zoé demain"),
    (&["Léa"], "j'ai vu Lea hier", "j'ai vu Léa hier"),
    (&["Anaïs"], "merci Anais", "merci Anaïs"),
    (&["Loïc"], "dis à Loic de venir", "dis à Loïc de venir"),
    (&["Noémie"], "Noemie arrive", "Noémie arrive"),
    (&["Clement"], "merci Clément", "merci Clement"),
    (&["Zoé"], "c'est Zoé.", "c'est Zoé."),
    // An accented everyday word is still what the user said.
    (&["Zoé"], "c'était déjà fait", "c'était déjà fait"),
    (&["Annie"], "une bonne année", "une bonne année"),
    // An everyday word is not an accented name it folds to.
    (&["Aimé"], "je t'aime", "je t'aime"),
    (&["Aimé"], "il aime le chocolat", "il aime le chocolat"),
    (&["Côme"], "come here", "come here"),
    (&["Côme"], "Come on!", "Come on!"),
    (&["Côme"], "dis à côme de venir", "dis à Côme de venir"),
    // An accented name is not another name that differs by a letter.
    (&["Léo"], "j'ai vu Léa hier", "j'ai vu Léa hier"),
    (&["Léa"], "j'ai vu Léo hier", "j'ai vu Léo hier"),
    (&["Théo"], "merci Théa", "merci Théa"),
    // An accent the custom word lacks makes a different word, unless
    // the transcriber capitalised it as a name.
    (
        &["Marie"],
        "je suis marié depuis deux ans",
        "je suis marié depuis deux ans",
    ),
    (&["Marie"], "elle est mariée", "elle est mariée"),
    (&["Marie"], "ils sont mariés", "ils sont mariés"),
    (&["Céline"], "elle est câline", "elle est câline"),
    (&["Clement"], "un hiver clément", "un hiver clément"),
    // A French elision stays in front of the custom word.
    (ELISION_WORDS, "j'adore l'iPhone", "j'adore l'iPhone"),
    (
        ELISION_WORDS,
        "le modèle d'Anthropic",
        "le modèle d'Anthropic",
    ),
    (
        ELISION_WORDS,
        "le modèle d’Anthropic.",
        "le modèle d’Anthropic.",
    ),
    (ELISION_WORDS, "un fichier d'excel", "un fichier d'Excel"),
    (ELISION_WORDS, "L'I phone est mort", "L'iPhone est mort"),
    (
        ELISION_WORDS,
        "la musique de D'Angelo",
        "la musique de D'Angelo",
    ),
    // An everyday word after the elision is what the user said.
    (
        ELISION_WORDS,
        "Bonjour, je m'appelle Marc",
        "Bonjour, je m'appelle Marc",
    ),
    (ELISION_WORDS, "il s'appelle Paul", "il s'appelle Paul"),
    (ELISION_WORDS, "on s'appelle demain", "on s'appelle demain"),
    (ELISION_WORDS, "je t'appelle", "je t'appelle"),
    (ELISION_WORDS, "il appelle demain", "il appelle demain"),
    (ELISION_WORDS, "envoie-moi l'email", "envoie-moi l'email"),
    (ELISION_WORDS, "un email", "un email"),
    (ELISION_WORDS, "il l'aura demain", "il l'aura demain"),
    (ELISION_WORDS, "elle l'aura vu", "elle l'aura vu"),
    (ELISION_WORDS, "la fin d'une époque", "la fin d'une époque"),
    (ELISION_WORDS, "demande à laura", "demande à Laura"),
    // A word made only of punctuation stays as written; words are
    // joined back with plain spaces.
    (&["Max"], "Tu viens ?", "Tu viens ?"),
    (&["Max"], "C'est ça !", "C'est ça !"),
    (&["Max"], "Note : demain", "Note : demain"),
    (&["Max"], "« Bonjour »", "« Bonjour »"),
    (&["Max"], "Tu viens\u{202f}?", "Tu viens ?"),
    (&["Max"], "R & D", "R & D"),
    (&["Max"], "this – that", "this – that"),
    (&["Max"], "wait …", "wait …"),
];

#[test]
fn custom_word_corpus() {
    let failures: Vec<String> = CORPUS
        .iter()
        .filter_map(|(words, input, expected)| {
            let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
            let (text, fixes) = apply_custom_words_counted(input, &words, THRESHOLD);
            let respaced = input.split_whitespace().collect::<Vec<_>>().join(" ");
            (text != *expected || (fixes > 0) != (text != respaced)).then(|| {
                format!("{words:?} {input:?}: got {text:?} ({fixes} fixes), want {expected:?}")
            })
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn test_apply_custom_words_fuzzy_match() {
    let text = "helo wrold";
    let custom_words = vec!["hello".to_string(), "world".to_string()];
    let result = apply_custom_words(text, &custom_words, 0.5);
    assert_eq!(result, "hello world");
}

#[test]
fn counted_custom_words_skip_matches_that_change_nothing() {
    let custom_words = vec!["hello".to_string(), "Murmur".to_string()];
    let (text, replacements) = apply_custom_words_counted("helo from Murmur", &custom_words, 0.5);
    assert_eq!((text.as_str(), replacements), ("hello from Murmur", 1));
    assert_eq!(
        apply_custom_words_counted("plain text", &[], 0.5),
        ("plain text".to_string(), 0)
    );
}

#[test]
fn test_preserve_case_pattern() {
    assert_eq!(preserve_case_pattern("HELLO", "world"), "WORLD");
    assert_eq!(preserve_case_pattern("Hello", "world"), "World");
    assert_eq!(preserve_case_pattern("hello", "WORLD"), "WORLD");
}

#[test]
fn test_extract_punctuation() {
    assert_eq!(extract_punctuation("hello"), ("", ""));
    assert_eq!(extract_punctuation("!hello?"), ("!", "?"));
    assert_eq!(extract_punctuation("...hello..."), ("...", "..."));
}

#[test]
fn test_extract_punctuation_uses_unicode_boundaries() {
    assert_eq!(extract_punctuation("你好。"), ("", "。"));
    assert_eq!(extract_punctuation("「你好」"), ("「", "」"));
    assert_eq!(extract_punctuation("你好！"), ("", "！"));
}

#[test]
fn test_filter_cleans_whitespace() {
    let text = "Hello    world   test";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "Hello world test");
}

#[test]
fn test_filter_trims() {
    let text = "  Hello world  ";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "Hello world");
}

#[test]
fn test_filter_preserves_valid_text() {
    let text = "This is a completely normal sentence.";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "This is a completely normal sentence.");
}

#[test]
fn test_filter_stutter_collapse() {
    let text = "w wh wh wh wh wh wh wh wh wh why";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "w wh why");
}

#[test]
fn test_filter_stutter_short_words() {
    let text = "I I I I think so so so so";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "I think so");
}

#[test]
fn test_filter_stutter_longer_words() {
    let text = "Check data doc doc doc doc documentation.";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "Check data doc documentation.");
}

#[test]
fn test_filter_stutter_mixed_case() {
    let text = "No NO no NO no";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "No");
}

#[test]
fn test_filter_stutter_preserves_two_repetitions() {
    let text = "no no is fine";
    let result = normalize_transcription_output(text);
    assert_eq!(result, "no no is fine");
}

#[test]
fn test_apply_custom_words_skips_cjk_fuzzy_matching() {
    let text = "你好。";
    let custom_words = vec!["你号".to_string()];
    let result = apply_custom_words(text, &custom_words, 1.0);
    assert_eq!(result, text);
}
