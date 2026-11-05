//! Instant, deterministic text fixes for English and French dictations:
//! fillers and spoken line breaks on every dictation, spoken punctuation on
//! the ones AI cleanup did not edit. They run only when the language is known,
//! because filler words are real words elsewhere ("um" in Portuguese, "er" in
//! German).

mod breaks;
mod fillers;
mod punctuation;
mod words;

use words::Words;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassLanguage {
    En,
    Fr,
}

const EN_STOPWORDS: &[&str] = &["the", "and", "is", "you", "to", "of", "that", "it"];
const FR_STOPWORDS: &[&str] = &[
    "le", "la", "les", "et", "est", "je", "pas", "que", "une", "des",
];

/// The language from the dictation setting, or in `auto` a stopword vote on
/// the text: at least 3 hits and twice the other side. Any other language,
/// or no clear vote, means no pass.
pub fn detect(setting: &str, text: &str) -> Option<PassLanguage> {
    let primary = setting.split(['-', '_']).next().unwrap_or_default();
    match primary.to_ascii_lowercase().as_str() {
        "en" => return Some(PassLanguage::En),
        "fr" => return Some(PassLanguage::Fr),
        "auto" => {}
        _ => return None,
    }
    let (mut en, mut fr) = (0, 0);
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let word = word.to_lowercase();
        en += usize::from(EN_STOPWORDS.contains(&word.as_str()));
        fr += usize::from(FR_STOPWORDS.contains(&word.as_str()));
    }
    if en >= 3 && en >= 2 * fr {
        Some(PassLanguage::En)
    } else if fr >= 3 && fr >= 2 * en {
        Some(PassLanguage::Fr)
    } else {
        None
    }
}

/// Fillers, then spoken line breaks. Runs on every dictation before cleanup.
pub fn before_cleanup(text: &str, language: PassLanguage) -> String {
    let mut words = Words::parse(text);
    fillers::remove(&mut words, language);
    breaks::apply(&mut words, language);
    words.join()
}

/// Spoken punctuation, for text AI cleanup did not edit. Cleanup reads the
/// same words in context ("trois virgule cinq"), so it never gets this.
pub fn raw_only(text: &str, language: PassLanguage) -> String {
    let mut words = Words::parse(text);
    punctuation::apply(&mut words, language);
    words.join()
}

const EN_DETERMINERS: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "my", "your", "his", "her", "its", "our",
    "their", "one", "another", "any", "each", "every", "no", "some", "what", "which",
];
const FR_DETERMINERS: &[&str] = &[
    "un", "une", "le", "la", "les", "ce", "cet", "cette", "ces", "mon", "ma", "mes", "ton", "ta",
    "tes", "son", "sa", "ses", "notre", "votre", "leur", "nos", "vos", "leurs", "chaque", "du",
    "des", "quel", "quelle", "aucun", "aucune",
];

/// Words that make a noun phrase out of the command words after them: "a
/// whole new line", "a brand new line", "toute nouvelle ligne". Only words
/// that never end a clause: an adjective like "big" also ends a list item
/// ("big comma small comma tiny"), and "completely" or "tout" end the clause
/// before a real command.
const EN_INTENSIFIERS: &[&str] = &["whole", "brand", "entire"];
const FR_INTENSIFIERS: &[&str] = &["toute", "véritable"];

/// Prepositions that never begin a line or a sentence: the command words
/// before them are a noun, "a question mark over the launch".
const EN_CONTINUATIONS: &[&str] = &["of", "over", "about", "than"];
const FR_CONTINUATIONS: &[&str] = &["de", "du", "sur"];

/// A spoken command is never part of a noun phrase: "a new line of
/// products", "a whole new line of shoes", "a big question mark over the
/// launch", "la virgule", "a full stop" keep their words. Punctuation
/// between the words ends the phrase: "item one, new line" is a command.
/// `len` is the number of words of the command at `index`.
fn inside_noun_phrase(words: &Words, index: usize, len: usize, language: PassLanguage) -> bool {
    after_opener(words, index, language) || before_continuation(words, index + len, language)
}

/// The command at `index` follows a determiner or an intensifier.
fn after_opener(words: &Words, index: usize, language: PassLanguage) -> bool {
    let (determiners, intensifiers) = match language {
        PassLanguage::En => (EN_DETERMINERS, EN_INTENSIFIERS),
        PassLanguage::Fr => (FR_DETERMINERS, FR_INTENSIFIERS),
    };
    only_spaces(words, index)
        && index
            .checked_sub(1)
            .and_then(|previous| words.key(previous))
            .is_some_and(|word| {
                determiners.contains(&word.as_str()) || intensifiers.contains(&word.as_str())
            })
}

/// The word at `next`, right after a command, is a lowercase continuation.
/// The comma does not use it: "yes comma of course" is a comma, and the
/// determiner rule already protects "the comma of the list".
fn before_continuation(words: &Words, next: usize, language: PassLanguage) -> bool {
    let continuations = match language {
        PassLanguage::En => EN_CONTINUATIONS,
        PassLanguage::Fr => FR_CONTINUATIONS,
    };
    next < words.items.len()
        && only_spaces(words, next)
        && !words.items[next]
            .word
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
        && words
            .key(next)
            .is_some_and(|word| continuations.contains(&word.as_str()))
}

fn only_spaces(words: &Words, at: usize) -> bool {
    words.items[at].gap.chars().all(|c| c == ' ')
}

#[cfg(test)]
mod tests;
