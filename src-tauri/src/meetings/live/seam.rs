//! Joins one track's live pieces into sentences. A piece cut inside speech
//! shares its last 2 s with the next one, so the words heard there twice are
//! kept once: from the shared word nearest the middle of that overlap, or,
//! when no word lines up, from the middle itself, dropping a phrase the next
//! piece repeats. A sentence is published only once no later piece can
//! change it.

use super::{LiveEntry, LiveEntryKind};
use crate::meetings::merge::Track as Source;
use crate::meetings::protocol::Word;
use crate::meetings::tokens::key;

const ANCHOR_MS: u64 = 300;
const REPEAT_WORDS: std::ops::RangeInclusive<usize> = 2..=12;

#[derive(Debug, Clone, PartialEq)]
pub struct HeardWord {
    pub text: String,
    pub key: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// The helper's words of a piece starting at `offset_ms` on the timeline.
pub fn heard(words: &[Word], offset_ms: u64) -> Vec<HeardWord> {
    let ms = |seconds: f64| offset_ms + (seconds.max(0.0) * 1000.0).round() as u64;
    words
        .iter()
        .map(|word| HeardWord {
            text: word.w.trim().to_string(),
            key: key(&word.w),
            start_ms: ms(word.s),
            end_ms: ms(word.e.max(word.s)),
        })
        .filter(|word| !word.key.is_empty())
        .collect()
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', '»', '”', '’'])
        .ends_with(['.', '?', '!', '…', '。', '？', '！'])
}

pub struct Draft {
    source: Source,
    words: Vec<HeardWord>,
    settled_ms: u64,
    open: bool,
}

impl Draft {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            words: Vec::new(),
            settled_ms: 0,
            open: false,
        }
    }

    pub fn settled_ms(&self) -> u64 {
        self.settled_ms
    }

    /// Takes the words of the piece starting at `start_ms`, whose first
    /// `overlap_ms` the previous piece heard too.
    pub fn accept(
        &mut self,
        start_ms: u64,
        overlap_ms: u64,
        mut words: Vec<HeardWord>,
        settled_ms: u64,
        open: bool,
    ) {
        if overlap_ms > 0 && !words.is_empty() {
            let seam_end = start_ms + overlap_ms;
            let middle = start_ms + overlap_ms / 2;
            match self.anchor(start_ms, seam_end, middle, &words) {
                Some((kept, from)) => {
                    self.words.truncate(kept);
                    words.drain(..from);
                }
                None => {
                    self.words.retain(|word| word.start_ms < middle);
                    words.retain(|word| word.start_ms >= middle);
                    let repeated = REPEAT_WORDS
                        .rev()
                        .find(|&count| {
                            count <= self.words.len()
                                && count <= words.len()
                                && self.words[self.words.len() - count..]
                                    .iter()
                                    .zip(&words[..count])
                                    .all(|(old, new)| old.key == new.key)
                        })
                        .unwrap_or(0);
                    words.drain(..repeated);
                }
            }
        }
        self.words.extend(words);
        self.settled_ms = self.settled_ms.max(settled_ms);
        self.open = open;
    }

    fn anchor(
        &self,
        seam_start: u64,
        seam_end: u64,
        middle: u64,
        words: &[HeardWord],
    ) -> Option<(usize, usize)> {
        let in_seam = |word: &HeardWord| {
            word.start_ms + ANCHOR_MS >= seam_start && word.start_ms <= seam_end + ANCHOR_MS
        };
        let mut best: Option<(u64, usize, usize)> = None;
        for (kept, old) in self.words.iter().enumerate().filter(|(_, w)| in_seam(w)) {
            for (from, new) in words.iter().enumerate().filter(|(_, w)| in_seam(w)) {
                if old.key != new.key || old.start_ms.abs_diff(new.start_ms) > ANCHOR_MS {
                    continue;
                }
                let distance = (old.start_ms + new.start_ms).abs_diff(2 * middle);
                if best.is_none_or(|(nearest, _, _)| distance < nearest) {
                    best = Some((distance, kept, from));
                }
            }
        }
        best.map(|(_, kept, from)| (kept, from))
    }

    /// The sentences no later piece can change.
    pub fn publish(&mut self) -> Vec<LiveEntry> {
        let mut entries = Vec::new();
        let mut first = 0;
        for (index, word) in self.words.iter().enumerate() {
            if word.start_ms >= self.settled_ms {
                break;
            }
            if ends_sentence(&word.text) {
                entries.push(self.entry(first..index + 1));
                first = index + 1;
            }
        }
        if !self.open
            && first < self.words.len()
            && self
                .words
                .iter()
                .all(|word| word.start_ms < self.settled_ms)
        {
            entries.push(self.entry(first..self.words.len()));
            first = self.words.len();
        }
        self.words.drain(..first);
        entries
    }

    /// Everything left: the track ended.
    pub fn finish(&mut self) -> Vec<LiveEntry> {
        self.open = false;
        self.settled_ms = u64::MAX;
        let entries = self.publish();
        self.settled_ms = 0;
        entries
    }

    /// The words so far, then a gap from `start_ms` to `end_ms` that the
    /// live text skips.
    pub fn gap(&mut self, start_ms: u64, end_ms: u64) -> Vec<LiveEntry> {
        let mut entries = self.finish();
        entries.push(LiveEntry {
            source: self.source,
            start_ms,
            end_ms,
            text: String::new(),
            kind: LiveEntryKind::Gap,
        });
        self.settled_ms = end_ms;
        entries
    }

    fn entry(&self, range: std::ops::Range<usize>) -> LiveEntry {
        let words = &self.words[range];
        LiveEntry {
            source: self.source,
            start_ms: words[0].start_ms,
            end_ms: words[words.len() - 1].end_ms,
            text: words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            kind: LiveEntryKind::Speech,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(text: &str, start_ms: u64) -> Vec<HeardWord> {
        text.split_whitespace()
            .enumerate()
            .map(|(index, word)| HeardWord {
                text: word.to_string(),
                key: key(word),
                start_ms: start_ms + 400 * index as u64,
                end_ms: start_ms + 400 * index as u64 + 300,
            })
            .collect()
    }

    fn texts(entries: &[LiveEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.text.as_str()).collect()
    }

    #[test]
    fn anchor_word_joins_hard_cut_without_duplicates() {
        let mut draft = Draft::new(Source::System);
        let first = said(
            "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twentyone twentytwo twentythree twentyfour twentyfive twentysix twentyseven twentyeight twentynine",
            0,
        );
        let first: Vec<_> = first.into_iter().filter(|w| w.start_ms < 12_000).collect();
        draft.accept(0, 0, first, 10_000, true);
        assert!(draft.publish().is_empty(), "no sentence ended yet");

        let mut second = vec![HeardWord {
            text: "ive".into(),
            key: "ive".into(),
            start_ms: 10_000,
            end_ms: 10_100,
        }];
        second.extend(said(
            "twentysix twentyseven twentyeight twentynine thirty. Done.",
            10_010,
        ));
        draft.accept(10_000, 2_000, second, 22_000, false);
        let entries = draft.publish();
        assert_eq!(entries.len(), 2);
        let words: Vec<&str> = entries[0].text.split(' ').collect();
        let mut unique = words.clone();
        unique.dedup();
        assert_eq!(words, unique, "a word was kept twice: {}", entries[0].text);
        assert!(entries[0].text.starts_with("one two"));
        assert!(entries[0]
            .text
            .contains("twentyfour twentyfive twentysix twentyseven"));
        assert!(entries[0].text.ends_with("twentynine thirty."));
        assert_eq!(entries[1].text, "Done.");
    }

    #[test]
    fn repeated_prefix_is_dropped_without_anchor() {
        let mut draft = Draft::new(Source::Mic);
        draft.accept(
            0,
            0,
            said("we ship on friday and then", 8_900),
            10_000,
            true,
        );
        draft.accept(
            10_000,
            2_000,
            said("and then we rest.", 11_400),
            22_000,
            false,
        );
        assert_eq!(
            texts(&draft.publish()),
            ["we ship on friday and then we rest."]
        );
    }

    #[test]
    fn sentence_waits_until_past_seam() {
        let mut draft = Draft::new(Source::Mic);
        let mut words = said("Yes.", 8_000);
        words.extend(said("We can start", 9_600));
        draft.accept(0, 0, words, 10_000, true);
        assert_eq!(texts(&draft.publish()), ["Yes."]);

        draft.accept(10_000, 2_000, said("start now.", 10_400), 14_000, false);
        assert_eq!(texts(&draft.publish()), ["We can start now."]);
    }

    #[test]
    fn finish_publishes_everything() {
        let mut draft = Draft::new(Source::System);
        draft.accept(0, 0, said("an unfinished thought", 10_500), 10_000, true);
        assert!(draft.publish().is_empty());
        let entries = draft.finish();
        assert_eq!(texts(&entries), ["an unfinished thought"]);
        assert_eq!(entries[0].start_ms, 10_500);
        assert_eq!(entries[0].end_ms, 11_600);

        let gap = draft.gap(20_000, 30_000);
        assert_eq!(gap.len(), 1);
        assert_eq!(gap[0].kind, LiveEntryKind::Gap);
    }
}
