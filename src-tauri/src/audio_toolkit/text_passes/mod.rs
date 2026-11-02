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

/// A spoken command is never the object of a determiner: "a new line of
/// products", "la virgule", "a full stop" keep their words. Punctuation
/// between them ends the phrase: "item one, new line" is a command.
fn follows_determiner(words: &Words, index: usize, language: PassLanguage) -> bool {
    let determiners = match language {
        PassLanguage::En => EN_DETERMINERS,
        PassLanguage::Fr => FR_DETERMINERS,
    };
    words.items[index].gap.chars().all(|c| c == ' ')
        && index
            .checked_sub(1)
            .and_then(|previous| words.key(previous))
            .is_some_and(|word| determiners.contains(&word.as_str()))
}

#[cfg(test)]
mod tests;
