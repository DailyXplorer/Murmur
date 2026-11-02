//! Parakeet Ultra writes `<unk>` where its training text had a character its
//! tokenizer cannot spell: quotes, brackets, dashes, `&`. v3 left those
//! characters out, and so does Murmur. The text and the timed words are
//! cleaned by the same rule, so they still split into the same words: a word
//! left empty goes, and one left with only closing punctuation joins the
//! word before it.

use crate::meetings::protocol::Word;

const UNKNOWN: &str = "<unk>";

/// `word` without `<unk>`, and whether it now joins the word before it.
fn clean(word: &str) -> Option<(String, bool)> {
    if !word.contains(UNKNOWN) {
        return Some((word.to_string(), false));
    }
    let cleaned = word.replace(UNKNOWN, "");
    if cleaned.is_empty() {
        return None;
    }
    let closing = cleaned
        .chars()
        .all(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '…' | ')' | '»' | '”'));
    Some((cleaned, closing))
}

pub fn text_without_unknown(text: &str) -> String {
    if !text.contains(UNKNOWN) {
        return text.to_string();
    }
    let mut words: Vec<String> = Vec::new();
    for (cleaned, joins) in text.split_whitespace().filter_map(clean) {
        match words.last_mut() {
            Some(before) if joins => before.push_str(&cleaned),
            _ => words.push(cleaned),
        }
    }
    words.join(" ")
}

pub fn words_without_unknown(words: Vec<Word>) -> Vec<Word> {
    if !words.iter().any(|word| word.w.contains(UNKNOWN)) {
        return words;
    }
    let mut kept: Vec<Word> = Vec::with_capacity(words.len());
    for word in words {
        let Some((cleaned, joins)) = clean(&word.w) else {
            continue;
        };
        match kept.last_mut() {
            Some(before) if joins => {
                before.w.push_str(&cleaned);
                before.e = before.e.max(word.e);
            }
            _ => kept.push(Word { w: cleaned, ..word }),
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heard_text_loses_the_unknown_tokens_and_keeps_its_punctuation() {
        for (heard, expected) in [
            (
                "Le terme <unk> civilisation <unk> est dérivé du mot latin <unk> civilis <unk> qui",
                "Le terme civilisation est dérivé du mot latin civilis qui",
            ),
            (
                "et <unk> civitas<unk> qui signifie",
                "et civitas qui signifie",
            ),
            (
                "comporte de nombreux éléments <unk>.",
                "comporte de nombreux éléments.",
            ),
            (
                "Apia is the capital of Samoa <unk> the town",
                "Apia is the capital of Samoa the town",
            ),
            ("<unk> Bonjour", "Bonjour"),
            ("<unk>", ""),
            ("Rien à  retirer ?", "Rien à  retirer ?"),
        ] {
            assert_eq!(text_without_unknown(heard), expected, "{heard}");
        }
    }

    #[test]
    fn timed_words_are_cleaned_like_the_text_they_come_with() {
        let word = |w: &str, s: f64| Word {
            w: w.into(),
            s,
            e: s + 0.5,
        };
        let text = "plus <unk> n'oubliez civitas<unk> éléments <unk>.";
        let words = vec![
            word("plus", 0.0),
            word("<unk>", 1.0),
            word("n'oubliez", 2.0),
            word("civitas<unk>", 3.0),
            word("éléments", 4.0),
            word("<unk>.", 5.0),
        ];
        let cleaned = words_without_unknown(words);
        assert_eq!(
            cleaned,
            [
                word("plus", 0.0),
                word("n'oubliez", 2.0),
                word("civitas", 3.0),
                Word {
                    w: "éléments.".into(),
                    s: 4.0,
                    e: 5.5,
                },
            ]
        );
        let text = text_without_unknown(text);
        assert_eq!(text.split_whitespace().count(), cleaned.len());
    }
}
