use super::words::Words;
use super::{follows_determiner, PassLanguage};

/// "period", "point", "deux points" and "colon" are left out: each is also an
/// everyday word.
const EN: &[(&[&str], &str)] = &[
    (&["comma"], ","),
    (&["question", "mark"], "?"),
    (&["exclamation", "mark"], "!"),
    (&["exclamation", "point"], "!"),
    (&["full", "stop"], "."),
];
const FR: &[(&[&str], &str)] = &[
    (&["point", "d'interrogation"], "?"),
    (&["point", "d'exclamation"], "!"),
    (&["point", "à", "la", "ligne"], ".\n"),
    (&["virgule"], ","),
];

const FR_NUMBERS: &[&str] = &[
    "zéro",
    "un",
    "une",
    "deux",
    "trois",
    "quatre",
    "cinq",
    "six",
    "sept",
    "huit",
    "neuf",
    "dix",
    "onze",
    "douze",
    "treize",
    "quatorze",
    "quinze",
    "seize",
    "dix-sept",
    "dix-huit",
    "dix-neuf",
    "vingt",
    "trente",
    "quarante",
    "cinquante",
    "soixante",
    "soixante-dix",
    "quatre-vingt",
    "quatre-vingts",
    "quatre-vingt-dix",
    "cent",
    "cents",
    "mille",
];

/// Turns spoken punctuation into the mark, attached to the word before it.
/// A command needs a word before it, and never follows a determiner ("a
/// full stop", "la virgule").
pub(super) fn apply(words: &mut Words, language: PassLanguage) {
    let marks = match language {
        PassLanguage::En => EN,
        PassLanguage::Fr => FR,
    };
    let mut index = 1;
    while index < words.items.len() {
        let found = marks.iter().find(|(phrase, mark)| {
            words.matches(index, phrase)
                && !follows_determiner(words, index, language)
                && !(*mark == "," && decimal(words, index, phrase.len(), language))
        });
        let Some((phrase, mark)) = found else {
            index += 1;
            continue;
        };
        let space_before = language == PassLanguage::Fr && matches!(*mark, "?" | "!");
        words.splice(index, index + phrase.len(), |before, after, has_next| {
            let before = before.trim_end_matches([' ', '\t', ',', '.', ';', ':', '!', '?']);
            let after = after.trim_start_matches([' ', '\t', ',', '.', ';', ':']);
            let space = if space_before { " " } else { "" };
            let then = if has_next && !mark.ends_with('\n') && !after.starts_with('\n') {
                " "
            } else {
                ""
            };
            format!("{before}{space}{mark}{then}{after}")
        });
        if *mark != "," {
            words.capitalize(index);
        }
    }
}

/// "trois virgule cinq" is a number, not a comma.
fn decimal(words: &Words, index: usize, len: usize, language: PassLanguage) -> bool {
    let number = |at: Option<usize>| {
        at.and_then(|at| words.key(at)).is_some_and(|word| {
            word.chars().all(|c| c.is_ascii_digit())
                || (language == PassLanguage::Fr && FR_NUMBERS.contains(&word.as_str()))
        })
    };
    number(index.checked_sub(1)) && number(Some(index + len))
}
