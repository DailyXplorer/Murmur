use super::symbols::after_article;
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

/// How many words ahead a bare "quote" looks for its closing command.
const PAIR_REACH: usize = 25;

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
                            && can_open(words, index + command.phrase.len())
                    }
                    Role::OpenPaired => {
                        open == 0
                            && !after_opener(words, index, language)
                            && can_open(words, index + command.phrase.len())
                            && closes_ahead(words, index + 2, commands)
                    }
                    Role::Close => open > 0 && index > 0 && !after_article(words, index, language),
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
                let before = before.trim_end_matches(trimmed);
                let after = after.trim_start_matches([' ', '\t', ',']);
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
/// writes after "open quote".
fn can_open(words: &Words, next: usize) -> bool {
    next < words.items.len()
        && words.items[next]
            .gap
            .chars()
            .all(|c| matches!(c, ' ' | '\t' | ','))
        && !words.items[next].gap.is_empty()
}

/// A closing command sits within reach from `from`.
fn closes_ahead(words: &Words, from: usize, commands: &[Command]) -> bool {
    (from..words.items.len().min(from + PAIR_REACH)).any(|at| {
        commands
            .iter()
            .any(|command| command.role == Role::Close && words.matches(at, command.phrase))
    })
}
