use super::words::Words;
use super::{follows_determiner, PassLanguage};

struct Command {
    phrase: &'static [&'static str],
    separator: &'static str,
    /// A command only where the sentence ends: "aller à la ligne de départ"
    /// keeps its words. After "point" it is the punctuation pass's "point à
    /// la ligne".
    at_sentence_end: bool,
}

const fn command(phrase: &'static [&'static str], separator: &'static str) -> Command {
    Command {
        phrase,
        separator,
        at_sentence_end: false,
    }
}

const LINE: &str = "\n";
const PARAGRAPH: &str = "\n\n";

const EN: &[Command] = &[
    command(&["new", "line"], LINE),
    command(&["new", "paragraph"], PARAGRAPH),
];
const FR: &[Command] = &[
    command(&["nouvelle", "ligne"], LINE),
    command(&["nouveau", "paragraphe"], PARAGRAPH),
    Command {
        phrase: &["à", "la", "ligne"],
        separator: LINE,
        at_sentence_end: true,
    },
];

/// Turns spoken line and paragraph breaks into breaks. "a new line of
/// products" keeps its words: a command never follows a determiner.
pub(super) fn apply(words: &mut Words, language: PassLanguage) {
    let commands = match language {
        PassLanguage::En => EN,
        PassLanguage::Fr => FR,
    };
    let mut index = 0;
    while index < words.items.len() {
        let found = commands.iter().find(|command| {
            words.matches(index, command.phrase)
                && !follows_determiner(words, index, language)
                && (!command.at_sentence_end
                    || (ends_a_sentence(words, index + command.phrase.len())
                        && index
                            .checked_sub(1)
                            .and_then(|previous| words.key(previous))
                            .as_deref()
                            != Some("point")))
        });
        let Some(command) = found else {
            index += 1;
            continue;
        };
        words.splice(index, index + command.phrase.len(), |before, after, _| {
            let before = before
                .trim_end_matches([' ', '\t'])
                .trim_end_matches(',')
                .trim_end_matches([' ', '\t']);
            let after = after.trim_start_matches([' ', '\t', ',', '.', ';', ':']);
            format!("{before}{}{after}", command.separator)
        });
        words.capitalize(index);
    }
}

fn ends_a_sentence(words: &Words, next: usize) -> bool {
    next >= words.items.len()
        || words
            .gap_after(next - 1)
            .contains(['.', ',', ';', ':', '!', '?', '\n'])
        || words.items[next]
            .word
            .chars()
            .next()
            .is_some_and(char::is_uppercase)
}
