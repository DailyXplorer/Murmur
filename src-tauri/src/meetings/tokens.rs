//! Words of a Codex transcript, with the key used to compare them across
//! recognisers and the sentence ends that decide where a turn may break.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The word as Codex wrote it, punctuation included.
    pub text: String,
    /// Lowercase letters and digits without accents; empty for punctuation.
    pub key: String,
    pub sentence_end: bool,
}

impl Token {
    /// How much speaking time the word is likely to take, in characters.
    pub fn weight(&self) -> usize {
        self.key.chars().count() + 1
    }
}

const SENTENCE_ENDS: [char; 7] = ['.', '?', '!', '…', '。', '？', '！'];
/// Closing marks that may follow a sentence end: `"Oui."` or `(voilà.)`.
const CLOSERS: [char; 8] = ['"', '\'', '’', '”', '»', ')', ']', '」'];

/// `qu'on` → `quon`, `C'est-à-dire` → `cestadire`, `2030,` → `2030`.
pub fn key(word: &str) -> String {
    word.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(CLOSERS).ends_with(SENTENCE_ENDS)
}

/// The words of `text`. CJK text, written without spaces, is also split
/// after each full stop so its sentences can be told apart.
pub fn tokenize(text: &str) -> Vec<Token> {
    text.split_whitespace()
        .flat_map(split_cjk_stops)
        .map(|word| Token {
            key: key(word),
            sentence_end: ends_sentence(word),
            text: word.to_string(),
        })
        .collect()
}

/// The tokens of `text` grouped by sentence. Text after the last stop is a
/// sentence too, so no word is ever left out.
pub fn sentences(text: &str) -> Vec<Vec<Token>> {
    let mut sentences = Vec::new();
    let mut current = Vec::new();
    for token in tokenize(text) {
        let end = token.sentence_end;
        current.push(token);
        if end {
            sentences.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        sentences.push(current);
    }
    sentences
}

/// The words of `tokens` with single spaces between them.
pub fn join(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|token| token.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `你好。我们开始吧。` → `你好。`, `我们开始吧。`; other words stay whole.
fn split_cjk_stops(word: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (index, c) in word.char_indices() {
        let end = index + c.len_utf8();
        if matches!(c, '。' | '？' | '！') && end < word.len() {
            pieces.push(&word[start..end]);
            start = end;
        }
    }
    pieces.push(&word[start..]);
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(text: &str) -> Vec<String> {
        tokenize(text).into_iter().map(|token| token.key).collect()
    }

    fn texts(sentences: &[Vec<Token>]) -> Vec<String> {
        sentences.iter().map(|sentence| join(sentence)).collect()
    }

    #[test]
    fn keys_ignore_case_accents_and_punctuation() {
        assert_eq!(
            keys("Déjà vu, ÉLÈVE naïve Ça Größe"),
            ["deja", "vu", "eleve", "naive", "ca", "große"]
        );
    }

    #[test]
    fn apostrophes_and_hyphens_join_the_word() {
        assert_eq!(key("qu'on"), "quon");
        assert_eq!(key("c’est"), key("c'est"));
        assert_eq!(key("C'est-à-dire"), "cestadire");
        assert_eq!(key("2030,"), "2030");
    }

    #[test]
    fn punctuation_alone_has_an_empty_key() {
        let tokens = tokenize("Alors – bon … voilà");
        assert_eq!(tokens[1].text, "–");
        assert_eq!(tokens[1].key, "");
        assert_eq!(tokens[3].key, "");
        assert_eq!(tokens[1].weight(), 1);
        assert_eq!(tokens[4].weight(), 6);
    }

    #[test]
    fn sentence_ends_are_flagged() {
        let ends: Vec<bool> = tokenize("Oui. Vraiment ? Non ! Bon… (voilà.) « Et » puis")
            .iter()
            .map(|token| token.sentence_end)
            .collect();
        assert_eq!(
            ends,
            [true, false, true, false, true, true, true, false, false, false, false]
        );
    }

    #[test]
    fn text_splits_into_sentences_without_losing_words() {
        let text = "  Bonjour à tous.  On commence ?\nOui, allons-y et puis";
        assert_eq!(
            texts(&sentences(text)),
            ["Bonjour à tous.", "On commence ?", "Oui, allons-y et puis"]
        );
        assert!(sentences("   ").is_empty());
    }

    #[test]
    fn cjk_text_splits_after_each_full_stop() {
        assert_eq!(
            texts(&sentences("你好。我们开始吧！好的")),
            ["你好。", "我们开始吧！", "好的"]
        );
        assert_eq!(key("你好。"), "你好");
    }
}
