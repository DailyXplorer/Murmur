use super::words::Words;
use super::{after_opener, before_continuation, only_spaces, PassLanguage};

/// How a symbol attaches to the words around it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Sticks to the word before and the word after: "my underscore name" is
    /// `my_name`. Needs both.
    Between,
    /// Like `Between`, and with no word before it in the clause only sticks
    /// to the one after: "at sign bob" is `@bob`.
    Affix,
    /// Sticks to the word after only: "hashtag release" is `#release`.
    Prefix,
}

struct Symbol {
    phrase: &'static [&'static str],
    mark: &'static str,
    kind: Kind,
    /// Words that make the phrase something else: "do dièse" is a note.
    not_after: &'static [&'static str],
}

const fn symbol(phrase: &'static [&'static str], mark: &'static str, kind: Kind) -> Symbol {
    Symbol {
        phrase,
        mark,
        kind,
        not_after: &[],
    }
}

/// A bare "slash", "dash", "dot", "point" or "hash" is left out of English:
/// each is also an everyday word ("slash prices", "a dash of salt", "hash
/// browns"). "tiret" is left out of French for the same reason.
const EN: &[Symbol] = &[
    symbol(&["at", "sign"], "@", Kind::Affix),
    symbol(&["hashtag"], "#", Kind::Prefix),
    symbol(&["hash", "sign"], "#", Kind::Prefix),
    symbol(&["underscore"], "_", Kind::Between),
    symbol(&["forward", "slash"], "/", Kind::Affix),
    symbol(&["back", "slash"], "\\", Kind::Affix),
    symbol(&["backslash"], "\\", Kind::Affix),
    symbol(&["hyphen"], "-", Kind::Between),
];

const FR_NOTES: &[&str] = &["do", "ré", "re", "mi", "fa", "sol", "la", "si"];

const FR: &[Symbol] = &[
    symbol(&["arobase"], "@", Kind::Affix),
    symbol(&["arobas"], "@", Kind::Affix),
    symbol(&["hashtag"], "#", Kind::Prefix),
    symbol(&["signe", "dièse"], "#", Kind::Prefix),
    symbol(&["mot-dièse"], "#", Kind::Prefix),
    Symbol {
        phrase: &["dièse"],
        mark: "#",
        kind: Kind::Prefix,
        not_after: FR_NOTES,
    },
    symbol(&["tiret", "bas"], "_", Kind::Between),
    symbol(&["tiret", "du", "huit"], "_", Kind::Between),
    symbol(&["tiret", "du", "8"], "_", Kind::Between),
    symbol(&["underscore"], "_", Kind::Between),
    symbol(&["slash"], "/", Kind::Affix),
    symbol(&["barre", "oblique", "inversée"], "\\", Kind::Affix),
    symbol(&["barre", "oblique"], "/", Kind::Affix),
    symbol(&["antislash"], "\\", Kind::Affix),
    symbol(&["anti-slash"], "\\", Kind::Affix),
    symbol(&["backslash"], "\\", Kind::Affix),
    symbol(&["trait", "d'union"], "-", Kind::Between),
];

/// Words that turn the symbol's name into a noun about the key itself: "the
/// underscore key", "ce symbole".
const EN_NAMED: &[&str] = &[
    "key",
    "keys",
    "character",
    "characters",
    "symbol",
    "symbols",
    "button",
];
const FR_NAMED: &[&str] = &[
    "touche",
    "touches",
    "caractère",
    "caractères",
    "symbole",
    "symboles",
    "bouton",
];

/// Articles and demonstratives. Possessives are left out: "my underscore
/// name" is an identifier.
const EN_ARTICLES: &[&str] = &["a", "an", "the", "this", "these", "those"];
const FR_ARTICLES: &[&str] = &[
    "un", "une", "le", "la", "les", "du", "des", "ce", "cet", "cette", "ces",
];

/// A word before that makes "underscore" a verb: "we underscore the risk",
/// "to underscore". "I" counts only capitalised, since `i` is an identifier.
const EN_VERB_CONTEXT: &[&str] = &[
    "to", "we", "you", "they", "he", "she", "it", "i", "will", "would", "can", "could", "should",
    "must", "may", "might", "shall", "not", "also", "just",
];

/// Turns spoken symbols into the character, stuck to the neighbouring words
/// as the symbol is written. A command needs the word after it, and never
/// sits in a noun phrase: "a hashtag", "the underscore key", "an at sign of
/// sorts" keep their words.
pub(super) fn apply(words: &mut Words, language: PassLanguage) {
    let symbols = match language {
        PassLanguage::En => EN,
        PassLanguage::Fr => FR,
    };
    let mut index = 0;
    while index < words.items.len() {
        let found = symbols.iter().find(|symbol| {
            words.matches(index, symbol.phrase) && fires(words, index, symbol, language)
        });
        let Some(symbol) = found else {
            index += 1;
            continue;
        };
        let joins_left = symbol.kind != Kind::Prefix && has_word_before(words, index);
        words.splice(index, index + symbol.phrase.len(), |before, _, _| {
            if joins_left {
                symbol.mark.to_string()
            } else {
                format!("{before}{}", symbol.mark)
            }
        });
    }
}

fn fires(words: &Words, index: usize, symbol: &Symbol, language: PassLanguage) -> bool {
    let next = index + symbol.phrase.len();
    if next >= words.items.len() || !only_spaces(words, next) {
        return false;
    }
    let (named, articles) = match language {
        PassLanguage::En => (EN_NAMED, EN_ARTICLES),
        PassLanguage::Fr => (FR_NAMED, FR_ARTICLES),
    };
    if words
        .key(next)
        .is_some_and(|word| named.contains(&word.as_str()))
        || before_continuation(words, next, language)
    {
        return false;
    }
    let before = index
        .checked_sub(1)
        .and_then(|previous| words.key(previous));
    if before
        .as_deref()
        .is_some_and(|word| named.contains(&word) || symbol.not_after.contains(&word))
    {
        return false;
    }
    match symbol.kind {
        Kind::Prefix => !after_opener(words, index, language),
        Kind::Between | Kind::Affix => {
            let has_before = has_word_before(words, index);
            if symbol.kind == Kind::Between && !has_before {
                return false;
            }
            !(has_before
                && before.as_deref().is_some_and(|word| {
                    articles.contains(&word)
                        || (language == PassLanguage::En
                            && EN_VERB_CONTEXT.contains(&word)
                            && !(word == "i" && words.items[index - 1].word == "i"))
                }))
        }
    }
}

/// A word sits right before the command, with only spaces between.
fn has_word_before(words: &Words, index: usize) -> bool {
    index > 0 && only_spaces(words, index) && !words.items[index].gap.is_empty()
}

/// The command at `index` follows an article or a demonstrative: "the open quote", "un slash".
pub(super) fn after_article(words: &Words, index: usize, language: PassLanguage) -> bool {
    let articles = match language {
        PassLanguage::En => EN_ARTICLES,
        PassLanguage::Fr => FR_ARTICLES,
    };
    index > 0
        && only_spaces(words, index)
        && words
            .key(index - 1)
            .is_some_and(|word| articles.contains(&word.as_str()))
}
