//! Spacing at an insertion point, independent of Accessibility and paste methods.

use regex::Regex;
use std::sync::OnceLock;

/// The last two grapheme bases and the last grapheme's emoji presentation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Before {
    pub last: char,
    pub previous: Option<char>,
    pub last_is_emoji: bool,
}

impl Before {
    /// Scalar-only context loses keycap emoji whose base is plain punctuation.
    pub fn from_graphemes(last: &str, previous: Option<&str>) -> Option<Self> {
        let base = |grapheme: &str| grapheme.chars().next().filter(|&c| c != '\u{FFFD}');
        let last_is_emoji = last.contains(['\u{FE0F}', '\u{20E3}']);
        let last = base(last)?;
        Some(Self {
            last,
            previous: previous.and_then(base),
            last_is_emoji: last_is_emoji || is_emoji(last),
        })
    }
}

/// Whether pasting `text` at this insertion point needs one leading space.
pub fn needs_leading_space(before: Before, text: &str) -> bool {
    let Some(first) = text.chars().next() else {
        return false;
    };
    if first.is_whitespace() || is_closing_start(first) {
        return false;
    }
    if is_unspaced_script(before.last) || is_unspaced_script(first) {
        return false;
    }
    if before.last == '’' && before.previous.is_some_and(char::is_alphabetic) {
        return false;
    }
    asks_for_space_after(before.last) || before.last_is_emoji
}

fn is_closing_start(c: char) -> bool {
    matches!(c, ',' | '.' | '?' | '!' | ';' | ':' | ')' | ']' | '}')
}

fn asks_for_space_after(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            '.' | ',' | ';' | ':' | '!' | '?' | '…' | ')' | ']' | '}' | '”' | '’' | '»'
        )
        || is_emoji(c)
}

fn is_unspaced_script(c: char) -> bool {
    static SCRIPTS: OnceLock<Regex> = OnceLock::new();
    SCRIPTS
        .get_or_init(|| {
            Regex::new(r"[\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}]")
                .expect("unspaced script character classes are valid")
        })
        .is_match(c.encode_utf8(&mut [0; 4]))
}

fn is_emoji(c: char) -> bool {
    static EMOJI: OnceLock<Regex> = OnceLock::new();
    EMOJI
        .get_or_init(|| {
            Regex::new(r"[\p{Extended_Pictographic}\p{Emoji_Presentation}]")
                .expect("emoji character classes are valid")
        })
        .is_match(c.encode_utf8(&mut [0; 4]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    fn before(text: &str) -> Before {
        let mut graphemes = text.graphemes(true).rev();
        Before::from_graphemes(graphemes.next().unwrap(), graphemes.next()).unwrap()
    }

    #[test]
    fn hex_word_and_closing_punctuation_cases_add_a_space() {
        for last in [
            'a', 'Z', '7', '.', ',', ';', ':', '!', '?', '…', ')', ']', '}', '”', '’', '»',
        ] {
            assert!(
                needs_leading_space(
                    Before {
                        last,
                        previous: None,
                        last_is_emoji: false,
                    },
                    "in progress"
                ),
                "after {last:?}"
            );
        }
    }

    #[test]
    fn hex_whitespace_opening_punctuation_and_symbols_stay_unchanged() {
        for last in [
            '@', '$', '#', '/', '-', '(', '[', '{', '“', '‘', '«', '\'', '"', ' ', '\n', '\t',
        ] {
            assert!(
                !needs_leading_space(
                    Before {
                        last,
                        previous: None,
                        last_is_emoji: false,
                    },
                    "in progress"
                ),
                "after {last:?}"
            );
        }
    }

    #[test]
    fn empty_spaced_or_closing_text_stays_unchanged() {
        for text in [
            "",
            " already spaced",
            "\nnext line",
            "\tindented",
            ", then more",
            ".",
            "?",
            "!",
            ";",
            ":",
            ")",
            "]",
            "}",
        ] {
            assert!(!needs_leading_space(before("word"), text), "{text:?}");
        }
    }

    #[test]
    fn accented_words_and_emoji_get_a_space() {
        assert!(needs_leading_space(before("déjà"), "été"));
        for text in [
            "de\u{301}ja\u{300}",
            "😀",
            "👍",
            "🇫🇷",
            "❤️",
            "☺️",
            "1\u{FE0F}\u{20E3}",
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
            "👨‍👩‍👧",
            "👍🏽",
        ] {
            assert!(
                needs_leading_space(before(text), "in progress"),
                "after {text:?}"
            );
        }
    }

    #[test]
    fn keycap_emoji_and_plain_symbols_have_distinct_spacing() {
        for (text, expected) in [
            ("#️⃣", true),
            ("*️⃣", true),
            ("0️⃣", true),
            ("#", false),
            ("*", false),
            ("#\u{FE0E}", false),
        ] {
            assert_eq!(
                needs_leading_space(before(text), "next"),
                expected,
                "after {text:?}"
            );
        }
    }

    #[test]
    fn a_typographic_apostrophe_after_a_letter_is_an_elision() {
        for text in [
            "l’",
            "qu’",
            "d’",
            "dell’",
            "l\u{301}’",
            "l\u{335}’",
            "qu\u{301}’",
        ] {
            assert!(!needs_leading_space(before(text), "été"), "{text}");
        }
        for text in [")’", " ’", "7’", "”"] {
            assert!(needs_leading_space(before(text), "next"), "{text}");
        }
    }

    #[test]
    fn unspaced_scripts_on_either_side_do_not_get_a_space() {
        for c in ['日', 'あ', 'カ', 'ｶ', '㐀', '𠀀', '𰀀'] {
            let text = c.to_string();
            assert!(!needs_leading_space(before(&text), "next"), "after {c}");
            assert!(!needs_leading_space(before("word"), &text), "before {c}");
            assert!(!needs_leading_space(before(&text), &text), "around {c}");
        }
        assert!(needs_leading_space(before("한"), "글"));
    }

    #[test]
    fn shared_kana_marks_and_cjk_punctuation_remain_unspaced() {
        for text in ["コーヒー", "ｺｰﾋｰ", "。", "、"] {
            assert!(
                !needs_leading_space(before(text), "next"),
                "unexpected space after {text:?}"
            );
            assert!(
                !needs_leading_space(before("word"), text),
                "unexpected space before {text:?}"
            );
        }
    }

    #[test]
    fn hangul_still_gets_a_space_on_either_side() {
        for text in ["한", "한국어"] {
            assert!(needs_leading_space(before(text), "next"), "after {text}");
            assert!(needs_leading_space(before("word"), text), "before {text}");
        }
    }
}
