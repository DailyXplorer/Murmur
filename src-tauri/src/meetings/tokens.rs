//! Words of a Codex transcript, with the key used to compare them across
//! recognisers and the sentence ends that decide where a turn may break.

use crate::insights::stats::is_cjk;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The word as Codex wrote it, punctuation included.
    pub text: String,
    /// Lowercase letters and digits without accents; empty for punctuation.
    pub key: String,
    pub sentence_end: bool,
    pub gap: Gap,
}

/// What came between a word and the one before it in the text Codex wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// The word starts a piece; the word before was transcribed apart.
    Start,
    Space,
    /// Nothing: CJK text, cut after a full stop.
    Glued,
}

impl Token {
    /// How much speaking time the word is likely to take, in characters.
    pub fn weight(&self) -> usize {
        self.key.chars().count() + 1
    }
}

const SENTENCE_ENDS: [char; 7] = ['.', '?', '!', '…', '。', '？', '！'];
const CJK_SENTENCE_ENDS: [char; 3] = ['。', '？', '！'];
/// Closing marks that may follow a sentence end: `"Oui."`, `(voilà.)` or
/// `「好的。」`.
const CLOSERS: [char; 12] = [
    '"', '\'', '’', '”', '»', ')', ']', '」', '』', '）', '】', '》',
];

/// `qu'on` → `quon`, `C'est-à-dire` → `cestadire`, `2030,` → `2030`.
pub fn key(word: &str) -> String {
    word.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Titles written before a name, as in "M. Dupont".
const TITLES: [&str; 7] = ["m", "mme", "mlle", "dr", "mr", "mrs", "st"];

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(CLOSERS).ends_with(SENTENCE_ENDS)
}

/// A closing mark written apart, as French spaces `« On verra. »`. A
/// straight quote is left out: it may as well open the next sentence.
fn is_lone_closer(word: &str) -> bool {
    word.chars()
        .all(|c| CLOSERS.contains(&c) && c != '"' && c != '\'')
}

fn is_title(word: &str) -> bool {
    word.strip_suffix('.')
        .is_some_and(|title| TITLES.contains(&title.to_lowercase().as_str()))
}

/// The words of `text`. CJK text, written without spaces, is also split
/// after each full stop so its sentences can be told apart.
pub fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for (index, spaced) in text.split_whitespace().enumerate() {
        for (piece, word) in split_cjk_stops(spaced).into_iter().enumerate() {
            let gap = match (index, piece) {
                (0, 0) => Gap::Start,
                (_, 0) => Gap::Space,
                _ => Gap::Glued,
            };
            tokens.push(Token {
                key: key(word),
                sentence_end: ends_sentence(word),
                text: word.to_string(),
                gap,
            });
        }
    }
    for index in 0..tokens.len() {
        // The sentence ends after the closing mark, not before it.
        if index > 0 && tokens[index - 1].sentence_end && is_lone_closer(&tokens[index].text) {
            tokens[index - 1].sentence_end = false;
            tokens[index].sentence_end = true;
        }
        // The name goes with its title.
        let before_a_name = tokens
            .get(index + 1)
            .is_some_and(|next| next.text.starts_with(char::is_uppercase));
        if before_a_name && is_title(&tokens[index].text) {
            tokens[index].sentence_end = false;
        }
    }
    tokens
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

/// The words of `tokens`, spaced as Codex wrote them.
pub fn join(tokens: &[Token]) -> String {
    let mut text = tokens
        .first()
        .map(|token| token.text.clone())
        .unwrap_or_default();
    for pair in tokens.windows(2) {
        text.push_str(match pair[1].gap {
            Gap::Space => " ",
            Gap::Glued => "",
            Gap::Start => between_pieces(&pair[0].text, &pair[1].text),
        });
        text.push_str(&pair[1].text);
    }
    text
}

/// Where two pieces transcribed apart meet: no space after a CJK full stop
/// or between two wide characters, as CSS Text drops a line break there.
fn between_pieces(left: &str, right: &str) -> &'static str {
    let after_stop = left.trim_end_matches(CLOSERS).ends_with(CJK_SENTENCE_ENDS);
    let both_wide =
        left.chars().next_back().is_some_and(wide) && right.chars().next().is_some_and(wide);
    if after_stop || both_wide {
        ""
    } else {
        " "
    }
}

/// Chinese and Japanese characters, their punctuation and full-width forms.
/// Hangul is not one: Korean puts spaces between words.
fn wide(c: char) -> bool {
    is_cjk(c) || matches!(c, '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FF9F}')
}

/// `「好的。」我们走吧！好的` → `「好的。」`, `我们走吧！`, `好的`. A closing
/// mark stays with the stop before it; other words stay whole.
fn split_cjk_stops(word: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut after_stop = false;
    for (index, c) in word.char_indices() {
        let stop = CJK_SENTENCE_ENDS.contains(&c);
        if after_stop && !stop && !CLOSERS.contains(&c) {
            pieces.push(&word[start..index]);
            start = index;
        }
        after_stop = stop || (after_stop && CLOSERS.contains(&c));
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
    fn spaced_closing_marks_and_titles_stay_in_their_sentence() {
        assert_eq!(
            texts(&sentences(
                "« On verra. » D'accord. Il a dit (oui. ) puis « Et » non. \
                 J'ai vu M. Dupont et Mme. Martin. Dr. Who. Il est là."
            )),
            [
                "« On verra. »",
                "D'accord.",
                "Il a dit (oui. )",
                "puis « Et » non.",
                "J'ai vu M. Dupont et Mme. Martin.",
                "Dr. Who.",
                "Il est là."
            ]
        );
        // A straight quote may open the next sentence as well as close one.
        assert_eq!(
            texts(&sentences("C'est fini. \" Bonjour \" dit-il.")),
            ["C'est fini.", "\" Bonjour \" dit-il."]
        );
        // A title at the end of a sentence still ends it.
        assert_eq!(
            texts(&sentences("Bonjour M. merci")),
            ["Bonjour M.", "merci"]
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

    #[test]
    fn a_cjk_closing_mark_stays_with_its_sentence() {
        let tokens = tokenize("「好的。」我们走吧！（好。）");
        let texts: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
        assert_eq!(texts, ["「好的。」", "我们走吧！", "（好。）"]);
        assert!(tokens.iter().all(|token| token.sentence_end));
    }

    #[test]
    fn words_are_joined_back_as_codex_wrote_them() {
        for text in [
            "我们开会吧。今天讨论预算。",
            "「好的。」我们走吧，好吗？",
            "“好的。”我们走吧。",
            "我们用 Claude (AI 助手) 来写。我们用Claude。OK，开始吧。",
            "大家好 今天开会",
            "Claudeを使います。GitHubも、使います。",
            "영화 《기생충》 정말 좋았어요. 반갑습니다.",
            "Bonjour à tous. On commence ?",
        ] {
            assert_eq!(join(&tokenize(text)), text);
        }
    }

    #[test]
    fn pieces_transcribed_apart_are_spaced_as_their_language_writes() {
        let joined = |left: &str, right: &str| join(&[tokenize(left), tokenize(right)].concat());
        assert_eq!(
            joined("我们开会吧。", "今天讨论预算。"),
            "我们开会吧。今天讨论预算。"
        );
        assert_eq!(joined("“好的。”", "OK，开始吧。"), "“好的。”OK，开始吧。");
        assert_eq!(joined("我们开会吧", "今天讨论"), "我们开会吧今天讨论");
        assert_eq!(joined("We use Claude", "来写。"), "We use Claude 来写。");
        assert_eq!(
            joined("안녕하세요.", "반갑습니다."),
            "안녕하세요. 반갑습니다."
        );
        assert_eq!(
            joined("Bonjour.", "On commence ?"),
            "Bonjour. On commence ?"
        );
    }
}
