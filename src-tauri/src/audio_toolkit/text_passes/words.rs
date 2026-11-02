//! A transcript as words and the gaps between them, so a pass can match
//! whole words and rewrite only the text around them.

/// One word and the text before it (spaces, punctuation, line breaks).
#[derive(Debug, Clone)]
pub(super) struct Item {
    pub gap: String,
    pub word: String,
}

#[derive(Debug, Clone)]
pub(super) struct Words {
    pub items: Vec<Item>,
    pub tail: String,
}

/// Apostrophes and hyphens join letters into one word ("d'exclamation",
/// "uh-oh"), so a pass never matches half of a compound.
fn is_joiner(c: char) -> bool {
    matches!(c, '\'' | '’' | '-')
}

impl Words {
    pub fn parse(text: &str) -> Self {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let in_word = |i: usize| {
            let c = chars[i].1;
            c.is_alphanumeric()
                || (is_joiner(c)
                    && i > 0
                    && i + 1 < chars.len()
                    && chars[i - 1].1.is_alphanumeric()
                    && chars[i + 1].1.is_alphanumeric())
        };
        let mut items = Vec::new();
        let mut gap_start = 0;
        let mut i = 0;
        while i < chars.len() {
            if !in_word(i) {
                i += 1;
                continue;
            }
            let start = chars[i].0;
            while i < chars.len() && in_word(i) {
                i += 1;
            }
            let end = chars.get(i).map_or(text.len(), |(at, _)| *at);
            items.push(Item {
                gap: text[gap_start..start].to_string(),
                word: text[start..end].to_string(),
            });
            gap_start = end;
        }
        Self {
            items,
            tail: text[gap_start..].to_string(),
        }
    }

    pub fn join(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            out.push_str(&item.gap);
            out.push_str(&item.word);
        }
        out.push_str(&self.tail);
        out
    }

    /// The word at `index`, lowercase, with a typographic apostrophe made
    /// straight.
    pub fn key(&self, index: usize) -> Option<String> {
        self.items
            .get(index)
            .map(|item| item.word.to_lowercase().replace('’', "'"))
    }

    /// Whether the words from `start` spell `phrase`, case-insensitively.
    pub fn matches(&self, start: usize, phrase: &[&str]) -> bool {
        phrase
            .iter()
            .enumerate()
            .all(|(offset, word)| self.key(start + offset).as_deref() == Some(*word))
    }

    /// The gap after the word at `index`: the next word's gap, or the tail.
    pub fn gap_after(&self, index: usize) -> &str {
        self.items
            .get(index + 1)
            .map_or(self.tail.as_str(), |item| item.gap.as_str())
    }

    /// Removes the words `start..end` and joins the gap before them to the
    /// gap after them with `join(before, after, a word follows)`.
    pub fn splice(
        &mut self,
        start: usize,
        end: usize,
        join: impl FnOnce(&str, &str, bool) -> String,
    ) {
        let before = std::mem::take(&mut self.items[start].gap);
        let has_next = end < self.items.len();
        let after = if has_next {
            std::mem::take(&mut self.items[end].gap)
        } else {
            std::mem::take(&mut self.tail)
        };
        let joined = join(&before, &after, has_next);
        if has_next {
            self.items[end].gap = joined;
        } else {
            self.tail = joined;
        }
        self.items.drain(start..end);
    }

    /// Whether the word at `index` begins a sentence: first in the text, or
    /// after a sentence end or a line break.
    pub fn starts_sentence(&self, index: usize) -> bool {
        let gap = self.items[index].gap.trim_end_matches([' ', '\t']);
        index == 0 || gap.ends_with(['.', '!', '?', '…', '\n'])
    }

    pub fn capitalize(&mut self, index: usize) {
        if let Some(item) = self.items.get_mut(index) {
            item.word = capitalized(&item.word);
        }
    }
}

pub(super) fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Lowercase, or capitalised on its first letter only: "um" and "Um", never
/// "UM" or "ER", which are abbreviations.
pub(super) fn spoken_case(word: &str) -> bool {
    let lower = word.to_lowercase();
    word == lower || word == capitalized(&lower)
}
