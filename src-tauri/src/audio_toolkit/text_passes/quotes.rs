use super::symbols::{after_article, EN_NAMED, FR_NAMED};
use super::words::Words;
use super::{after_opener, PassLanguage};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// "open quote": fires wherever it is said.
    Open,
    /// "quote …": also an everyday word, so it fires only with a closing
    /// command a few words ahead.
    OpenPaired,
    Close,
}

struct Command {
    phrase: &'static [&'static str],
    role: Role,
}

const fn command(phrase: &'static [&'static str], role: Role) -> Command {
    Command { phrase, role }
}

/// Bare "quote" is also a price quote and a verb, so it opens only when a
/// "unquote" or "end quote" follows. "quote unquote" with nothing between is
/// the scare-quote idiom and keeps its words.
const EN: &[Command] = &[
    command(&["open", "quote"], Role::Open),
    command(&["open", "quotes"], Role::Open),
    command(&["begin", "quote"], Role::Open),
    command(&["quote"], Role::OpenPaired),
    command(&["close", "quote"], Role::Close),
    command(&["close", "quotes"], Role::Close),
    command(&["end", "quote"], Role::Close),
    command(&["unquote"], Role::Close),
];
const FR: &[Command] = &[
    command(&["ouvrez", "les", "guillemets"], Role::Open),
    command(&["ouvre", "les", "guillemets"], Role::Open),
    command(&["ouvrir", "les", "guillemets"], Role::Open),
    command(&["fermez", "les", "guillemets"], Role::Close),
    command(&["ferme", "les", "guillemets"], Role::Close),
    command(&["fermer", "les", "guillemets"], Role::Close),
];

/// How many words ahead an opening command looks for a closing one, so a
/// lone "open quote" leaves the text alone instead of a dangling mark.
const PAIR_REACH: usize = 40;

/// Quote marks as typeset in the language: curly double quotes in English,
/// guillemets with a space inside in French (as `text.rs` writes them).
fn marks(language: PassLanguage) -> (&'static str, &'static str) {
    match language {
        PassLanguage::En => ("“", "”"),
        PassLanguage::Fr => ("« ", " »"),
    }
}

/// Turns spoken quote commands into quote marks: the opening mark sticks to
/// the word after it, the closing one to the word before it. A closing
/// command needs an opening one said before it, and neither sits in a noun
/// phrase: "the open quote character", "la fermeture des guillemets" keep
/// their words.
pub(super) fn apply(words: &mut Words, language: PassLanguage) {
    let commands = match language {
        PassLanguage::En => EN,
        PassLanguage::Fr => FR,
    };
    let (open_mark, close_mark) = marks(language);
    let mut open = 0usize;
    let mut index = 0;
    while index < words.items.len() {
        let found = commands.iter().find(|command| {
            words.matches(index, command.phrase)
                && match command.role {
                    Role::Open => {
                        !after_article(words, index, language)
                            && can_open(words, index + command.phrase.len(), language)
                            && closes_ahead(words, index + command.phrase.len(), commands, language)
                    }
                    Role::OpenPaired => {
                        open == 0
                            && !after_opener(words, index, language)
                            && can_open(words, index + command.phrase.len(), language)
                            && closes_ahead(words, index + 1, commands, language)
                    }
                    Role::Close => {
                        open > 0 && can_close(words, index, command.phrase.len(), language)
                    }
                }
        });
        let Some(command) = found else {
            index += 1;
            continue;
        };
        let end = index + command.phrase.len();
        if command.role == Role::Close {
            open -= 1;
            let trimmed: &[char] = if language == PassLanguage::Fr {
                &[' ', '\t', ',']
            } else {
                &[' ', '\t']
            };
            words.splice(index, end, |before, after, has_next| {
                let said_comma = before.trim_end_matches([' ', '\t']).ends_with(',');
                let before = before.trim_end_matches(trimmed);
                let after = after.trim_start_matches([' ', '\t']);
                let after = match after.strip_prefix(',') {
                    Some(rest) if said_comma || language == PassLanguage::Fr => {
                        rest.trim_start_matches([' ', '\t'])
                    }
                    _ => after,
                };
                let after = if has_next && after.is_empty() {
                    " "
                } else {
                    after
                };
                format!("{before}{close_mark}{after}")
            });
        } else {
            open += 1;
            words.splice(index, end, |before, _, _| format!("{before}{open_mark}"));
        }
    }
}

/// A word follows with only spaces or commas between, the pause an engine
/// writes after "open quote", and it does not name the key or character.
fn can_open(words: &Words, next: usize, language: PassLanguage) -> bool {
    next < words.items.len()
        && !words.items[next].gap.is_empty()
        && words.items[next]
            .gap
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | ','))
        && !names_the_mark(words, next, language)
}

/// The command of `len` words at `index` can close: a word before it on the
/// same line, and not a noun phrase.
fn can_close(words: &Words, index: usize, len: usize, language: PassLanguage) -> bool {
    index > 0
        && !words.items[index].gap.contains(['\n', '\r'])
        && !after_article(words, index, language)
        && !names_the_mark(words, index + len, language)
}

/// The word at `at` turns the command into a noun phrase: "close quote key".
fn names_the_mark(words: &Words, at: usize, language: PassLanguage) -> bool {
    let named = match language {
        PassLanguage::En => EN_NAMED,
        PassLanguage::Fr => FR_NAMED,
    };
    words
        .key(at)
        .is_some_and(|word| named.contains(&word.as_str()))
}

/// A closing command that would itself fire sits within reach from `from`.
fn closes_ahead(words: &Words, from: usize, commands: &[Command], language: PassLanguage) -> bool {
    (from..words.items.len().min(from + PAIR_REACH)).any(|at| {
        commands.iter().any(|command| {
            command.role == Role::Close
                && words.matches(at, command.phrase)
                && at > from
                && can_close(words, at, command.phrase.len(), language)
        })
    })
}
