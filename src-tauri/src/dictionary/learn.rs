//! Learning dictionary words from the user's own corrections. Murmur knows
//! what it pasted; the user's edit of that span is compared word by word,
//! and a word is learned only when one or two pasted plain words were
//! replaced by a similar-sounding or similarly spelled term that looks like
//! a name. A learned word is only a vocabulary hint, like Wispr Flow's
//! auto-learned names and VoiceInk's vocabulary: it never rewrites text.
//! Ordinary lowercase words are never learned: changing "their" to "there"
//! is grammar, not vocabulary. Nor is a correction whose misheard side is
//! an everyday word ("cloud" -> "Claude"), an added suffix, or a word the
//! user kept elsewhere in the same text. Nor is swapping one term for
//! another ("Q3" -> "Q4", "iOS" -> "macOS", "John" -> "Joan"): the
//! transcript already held a name, so the user changed what they meant.
//! Chinese and Japanese, written without spaces, are never learned: only a
//! Latin term fixed inside them, which is cut out of the text around it. A
//! possessive or a French elision kept on both sides is cut off the same way.

use crate::insights::stats::is_cjk;
use natural::phonetics::soundex;
use std::collections::HashSet;
use std::sync::OnceLock;
use strsim::normalized_levenshtein;

/// Above this many words the diff is skipped; nobody fixes a name in an essay
/// by retyping it, and the alignment is quadratic.
const MAX_WORDS: usize = 400;
/// Changing more than this share of the dictation is a rewrite, not a fix.
const MAX_CHANGED_SHARE: f64 = 0.4;
const MIN_SIMILARITY: f64 = 0.5;
const MAX_LEARNED_PER_DICTATION: usize = 3;
const MAX_WORD_CHARS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// The spelling the user wants.
    pub word: String,
    /// What Murmur wrote instead.
    pub heard_as: String,
}

/// The part of `after` that replaced `inserted` inside `before`, when the
/// surrounding text is untouched. `before` is the field's value right after
/// the paste and must contain `inserted`.
pub fn edited_span<'a>(before: &str, inserted: &str, after: &'a str) -> Option<&'a str> {
    let inserted = inserted.trim();
    if inserted.is_empty() {
        return None;
    }
    let start = before.rfind(inserted)?;
    let prefix = &before[..start];
    let suffix = &before[start + inserted.len()..];
    if after.len() < prefix.len() + suffix.len() {
        return None;
    }
    after.strip_prefix(prefix)?.strip_suffix(suffix)
}

/// Compares what Murmur pasted with what the user left in its place and
/// returns the words to learn. An emptied or mostly replaced span teaches
/// nothing: chat apps clear the field when the message is sent, the next
/// message then takes its place, and a rewrite fixes no word.
pub fn review(inserted: &str, edited: &str) -> Vec<Correction> {
    let old = words(inserted);
    let new = words(edited);
    if old.is_empty() || new.is_empty() || old.len() > MAX_WORDS || new.len() > MAX_WORDS {
        return Vec::new();
    }

    let hunks: Vec<Hunk> = diff(&old, &new)
        .into_iter()
        .filter(|hunk| !is_cosmetic(&old[hunk.old.clone()], &new[hunk.new.clone()]))
        .collect();
    let words_changed: usize = hunks.iter().map(|hunk| hunk.old.len()).sum();
    if words_changed as f64 > (old.len() as f64 * MAX_CHANGED_SHARE).max(2.0) {
        return Vec::new();
    }

    let mut learned: Vec<Correction> = Vec::new();
    for hunk in hunks {
        let replaced = &old[hunk.old.clone()];
        let replacement = &new[hunk.new.clone()];
        if !(1..=2).contains(&replaced.len()) || replacement.len() != 1 {
            continue;
        }
        let replaced_core = replaced
            .iter()
            .map(|w| core(w))
            .collect::<Vec<_>>()
            .join(" ");
        let replacement_core = core(replacement[0]);
        let (heard_as, word) = without_clitics(&replaced_core, replacement_core);
        // A name after a stripped elision does not open the sentence: the
        // capital of "D'Anthropic" is the user's, not the sentence's.
        let sentence_start = replacement_core.starts_with(word)
            && (hunk.new.start == 0 || ends_sentence(new[hunk.new.start - 1]));
        let heard_at_sentence_start = replaced_core.starts_with(heard_as)
            && (hunk.old.start == 0 || ends_sentence(old[hunk.old.start - 1]));
        let heard_a_term = heard_as
            .split_whitespace()
            .enumerate()
            .any(|(i, w)| looks_like_a_term(w, i == 0 && heard_at_sentence_start));
        if looks_like_a_term(word, sentence_start)
            && !heard_a_term
            && !said_on_purpose(heard_as, word)
            && sounds_alike(heard_as, word)
            && !only_inflected(heard_as, word)
            && !only_extended(heard_as, word)
            && !kept_elsewhere(heard_as, &new)
            && !learned.iter().any(|known| known.word == word)
        {
            learned.push(Correction {
                word: word.to_string(),
                heard_as: heard_as.to_string(),
            });
        }
        if learned.len() == MAX_LEARNED_PER_DICTATION {
            break;
        }
    }
    learned
}

/// The words of `text`, split at spaces and where Chinese or Japanese meets
/// other letters or digits: `我们用cloud` → `我们用`, `cloud`.
fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().flat_map(split_scripts).collect()
}

fn split_scripts(word: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut script = None;
    for (index, c) in word.char_indices().filter(|(_, c)| c.is_alphanumeric()) {
        let cjk = is_cjk(c);
        if script.is_some_and(|before| before != cjk) {
            pieces.push(&word[start..index]);
            start = index;
        }
        script = Some(cjk);
    }
    pieces.push(&word[start..]);
    pieces
}

#[derive(Debug, PartialEq)]
struct Hunk {
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
}

/// Word-level longest-common-subsequence diff. Words match on their
/// punctuation-stripped form, so a moved comma is not a change.
fn diff(old: &[&str], new: &[&str]) -> Vec<Hunk> {
    let (n, m) = (old.len(), new.len());
    let same = |i: usize, j: usize| core(old[i]) == core(new[j]);
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if same(i, j) {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut hunks = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut old_start, mut new_start) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && same(i, j) {
            push_hunk(&mut hunks, old_start..i, new_start..j);
            i += 1;
            j += 1;
            old_start = i;
            new_start = j;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            j += 1;
        } else {
            i += 1;
        }
    }
    push_hunk(&mut hunks, old_start..i, new_start..j);
    hunks
}

/// Records a change. Equal-length runs are split word by word, so three
/// adjacent fixed names are three one-word corrections.
fn push_hunk(hunks: &mut Vec<Hunk>, old: std::ops::Range<usize>, new: std::ops::Range<usize>) {
    if old.is_empty() && new.is_empty() {
        return;
    }
    if old.len() == new.len() {
        hunks.extend(old.zip(new).map(|(i, j)| Hunk {
            old: i..i + 1,
            new: j..j + 1,
        }));
    } else {
        hunks.push(Hunk { old, new });
    }
}

/// A word without the punctuation around it; inner symbols such as the `&`
/// of "R&D" or the `-` of "GPT-4" are part of the word.
fn core(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
}

/// French elisions, written against the next word: "d'Anthropic".
const ELISIONS: [&str; 12] = [
    "c", "d", "j", "l", "m", "n", "s", "t", "qu", "jusqu", "lorsqu", "puisqu",
];

/// A clitic on both sides is grammar around the name, not part of it:
/// "clawd's" -> "Claude's" fixes "Claude", and "d'antropic" ->
/// "d'Anthropic" fixes "Anthropic", also capitalised at sentence start
/// ("D'Anthropic"). "O'Brien" stays whole: "o" is not an elision.
fn without_clitics<'a>(heard: &'a str, word: &'a str) -> (&'a str, &'a str) {
    let elided = |text: &'a str| {
        text.split_once(['\'', '’'])
            .filter(|(elision, _)| ELISIONS.contains(&elision.to_lowercase().as_str()))
    };
    let (heard, word) = match (elided(heard), elided(word)) {
        (Some((a, heard)), Some((b, word))) if a.eq_ignore_ascii_case(b) => (heard, word),
        _ => (heard, word),
    };
    let possessive = |text: &'a str| text.strip_suffix("'s").or_else(|| text.strip_suffix("’s"));
    match (possessive(heard), possessive(word)) {
        (Some(heard), Some(word)) => (heard, word),
        _ => (heard, word),
    }
}

fn letters(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Case-only and punctuation-only edits teach nothing about vocabulary.
/// Joining or splitting words ("Chat gpt" -> "ChatGPT") is not cosmetic.
fn is_cosmetic(old: &[&str], new: &[&str]) -> bool {
    let words = |tokens: &[&str]| -> Vec<String> {
        tokens
            .iter()
            .map(|token| letters(token))
            .filter(|key| !key.is_empty())
            .collect()
    };
    words(old) == words(new)
}

fn ends_sentence(word: &str) -> bool {
    word.ends_with(['.', '!', '?', '…', ':'])
}

/// Names, brands and jargon: a capital that is not just sentence case, a
/// digit, or an inner symbol. A run of Chinese or Japanese is not one: its
/// words cannot be told apart.
fn looks_like_a_term(word: &str, sentence_start: bool) -> bool {
    let chars = word.chars().count();
    if !(2..=MAX_WORD_CHARS).contains(&chars)
        || !word.chars().any(char::is_alphabetic)
        || word.chars().any(is_cjk)
    {
        return false;
    }
    let inner_capital = word.chars().skip(1).any(char::is_uppercase);
    let leading_capital = word.chars().next().is_some_and(char::is_uppercase);
    let digit_or_symbol = word
        .chars()
        .any(|c| c.is_ascii_digit() || "&-+._".contains(c));
    inner_capital || digit_or_symbol || (leading_capital && !sentence_start)
}

/// Same letters with different spacing count as alike: cosmetic edits were
/// already filtered out, so equal keys mean words were joined.
fn sounds_alike(heard: &str, word: &str) -> bool {
    let (heard, word) = (letters(heard), letters(word));
    if heard.is_empty() {
        return false;
    }
    if heard == word {
        return true;
    }
    let ascii_letters = |key: &str| key.chars().all(|c| c.is_ascii_alphabetic());
    normalized_levenshtein(&heard, &word) >= MIN_SIMILARITY
        || (ascii_letters(&heard) && ascii_letters(&word) && soundex(&heard, &word))
}

/// Everyday English and French words and common first names, lowercase.
fn is_common_word(word: &str) -> bool {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS
        .get_or_init(|| include_str!("common_words.txt").lines().collect())
        .contains(word.to_lowercase().as_str())
}

/// A misheard form made only of everyday words or first names is also
/// something the user says on purpose, so nothing is learned from it. Two or
/// more of them spelling the word's exact letters ("open ai" -> "OpenAI") are
/// the exception: that sequence only ever means the joined word.
fn said_on_purpose(heard_as: &str, word: &str) -> bool {
    let words: Vec<&str> = heard_as.split_whitespace().collect();
    let joined_spelling = words.len() > 1 && letters(heard_as) == letters(word);
    words.iter().all(|word| is_common_word(word)) && !joined_spelling
}

/// "Sarah" -> "Sarah's" or "report" -> "reports" changes the grammar of a
/// word the user already had.
fn only_inflected(heard: &str, word: &str) -> bool {
    let (heard, word) = (letters(heard), letters(word));
    let (short, long) = if heard.len() <= word.len() {
        (heard, word)
    } else {
        (word, heard)
    };
    short != long && long.starts_with(&short) && long.chars().count() - short.chars().count() <= 2
}

/// "stripe" -> "stripe.com" or "sarah" -> "sarah@example.com" adds to a
/// word the user said rightly; it was not misheard.
fn only_extended(heard: &str, word: &str) -> bool {
    word.to_lowercase()
        .strip_prefix(&heard.to_lowercase())
        .is_some_and(|rest| rest.starts_with(|c: char| !c.is_alphanumeric()))
}

/// A misheard form the user left untouched elsewhere in the text is one
/// they meant.
fn kept_elsewhere(heard_as: &str, edited: &[&str]) -> bool {
    let heard: Vec<String> = heard_as.split_whitespace().map(letters).collect();
    !heard.is_empty()
        && edited.windows(heard.len()).any(|window| {
            window
                .iter()
                .map(|word| letters(word))
                .eq(heard.iter().cloned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn learned(inserted: &str, edited: &str) -> Vec<(String, String)> {
        review(inserted, edited)
            .into_iter()
            .map(|c| (c.word, c.heard_as))
            .collect()
    }

    fn pair(word: &str, heard_as: &str) -> (String, String) {
        (word.to_string(), heard_as.to_string())
    }

    #[test]
    fn a_respelled_name_is_learned() {
        assert_eq!(
            learned(
                "I talked to clawd about it.",
                "I talked to Claude about it."
            ),
            vec![pair("Claude", "clawd")]
        );
        assert_eq!(
            learned(
                "deploy it on cooper netties today",
                "deploy it on Kubernetes today"
            ),
            vec![pair("Kubernetes", "cooper netties")]
        );
        assert_eq!(
            learned("send it to the r and d team", "send it to the R&D team"),
            Vec::<(String, String)>::new()
        );
    }

    #[test]
    fn product_names_with_inner_capitals_or_digits_are_learned_at_sentence_start() {
        assert_eq!(
            learned("Chat gpt is down again.", "ChatGPT is down again."),
            vec![pair("ChatGPT", "Chat gpt")]
        );
        assert_eq!(
            learned(
                "We moved to gpt for yesterday.",
                "We moved to GPT-4 yesterday."
            ),
            vec![pair("GPT-4", "gpt for")]
        );
    }

    #[test]
    fn grammar_fixes_of_ordinary_words_are_not_learned() {
        assert!(learned("I think their right.", "I think they're right.").is_empty());
        assert!(learned("Their right.", "There right.").is_empty());
    }

    #[test]
    fn a_common_word_is_never_learned_as_a_mishearing() {
        assert!(learned("talk to the cloud", "talk to the Claude").is_empty());
        assert!(learned("ask and about it", "ask Ann about it").is_empty());
        assert!(learned("please mark it", "please Marc it").is_empty());
        assert!(learned("envoie le au bureau", "envoie le au Burrow").is_empty());
    }

    #[test]
    fn swapping_one_name_for_another_is_a_content_edit() {
        assert!(learned("meet John at 3", "meet Joan at 3").is_empty());
        assert!(learned("ask Mark about it", "ask Marc about it").is_empty());
    }

    #[test]
    fn an_added_suffix_is_not_a_new_word() {
        assert!(learned("Sarah report is due", "Sarah's report is due").is_empty());
        assert!(learned("ping Kuberneet now", "ping Kuberneets now").is_empty());
    }

    #[test]
    fn swapping_one_term_for_another_is_not_learned() {
        assert!(learned("Q3 revenue was strong", "Q4 revenue was strong").is_empty());
        assert!(learned("we moved to GPT-4 today", "we moved to GPT-5 today").is_empty());
        assert!(learned("export it as mp3", "export it as mp4").is_empty());
        assert!(learned("it runs on iOS", "it runs on macOS").is_empty());
        assert!(learned("open it in Figma", "open it in FigJam").is_empty());
        assert!(learned("meet Smith tomorrow", "meet Smyth tomorrow").is_empty());
        assert!(learned("I talked to Clawd today", "I talked to Claude today").is_empty());
    }

    #[test]
    fn a_joined_pair_of_common_words_is_learned() {
        assert_eq!(
            learned(
                "we switched to open ai last week",
                "we switched to OpenAI last week"
            ),
            vec![pair("OpenAI", "open ai")]
        );
    }

    #[test]
    fn a_heard_as_the_user_kept_elsewhere_is_not_a_mishearing() {
        assert!(learned(
            "I told clawd and clawd agreed",
            "I told Claude and clawd agreed"
        )
        .is_empty());
    }

    #[test]
    fn case_only_and_punctuation_only_edits_are_ignored() {
        assert!(learned("call paris today", "call Paris, today!").is_empty());
        assert!(learned("send the e-mail", "send the email").is_empty());
    }

    #[test]
    fn a_rewritten_sentence_teaches_nothing() {
        assert!(learned(
            "the quarterly numbers look good for marta",
            "Revenue grew a lot this quarter, Martha",
        )
        .is_empty());
    }

    #[test]
    fn a_sent_chat_message_or_the_next_reply_teaches_nothing() {
        let inserted = "see you at five";
        let edited = edited_span(inserted, inserted, "").unwrap();
        assert!(review(inserted, edited).is_empty());
        assert!(review("ok", "  ").is_empty());
        let next_message = edited_span(inserted, inserted, "ok thanks").unwrap();
        assert!(review(inserted, next_message).is_empty());
        assert!(review("Sounds good", "see you tomorrow").is_empty());
        assert!(review("Thanks", "ok").is_empty());
        assert!(review("Thanks", "Thomas").is_empty());
    }

    #[test]
    fn a_word_extended_into_a_domain_or_address_is_not_learned() {
        assert!(learned("search it on google", "search it on google.com").is_empty());
        assert!(learned("pay with stripe today", "pay with stripe.com today").is_empty());
        assert!(learned("deploy on vercel now", "deploy on vercel.app now").is_empty());
    }

    #[test]
    fn an_unrelated_replacement_is_not_learned() {
        assert!(learned("I met Bob at the station", "I met Xavier at the station").is_empty());
    }

    #[test]
    fn added_or_deleted_words_are_not_vocabulary() {
        assert!(learned("ship it", "ship it Monday").is_empty());
        assert!(learned("ship it now please", "ship it please").is_empty());
    }

    #[test]
    fn chinese_and_japanese_runs_are_never_learned_as_words() {
        assert!(learned("会议三点开始。", "会议3点开始。").is_empty());
        assert!(learned("我们用cloud code", "我们用Claude Code").is_empty());
        assert!(learned("ぼくはクロードを使う", "ぼくはClaudeを使う").is_empty());
    }

    #[test]
    fn a_name_fixed_inside_chinese_or_japanese_is_learned_alone() {
        assert_eq!(
            learned("我们用clawd code", "我们用Claude code"),
            vec![pair("Claude", "clawd")]
        );
        assert_eq!(
            learned("時々clawdを使う", "時々Claudeを使う"),
            vec![pair("Claude", "clawd")]
        );
    }

    #[test]
    fn a_possessive_or_an_elision_on_both_sides_is_not_part_of_the_name() {
        assert_eq!(
            learned("I read clawd's answer.", "I read Claude's answer."),
            vec![pair("Claude", "clawd")]
        );
        assert_eq!(
            learned("I read clawd’s answer.", "I read Claude’s answer."),
            vec![pair("Claude", "clawd")]
        );
        assert!(learned("I read the cloud's answer", "I read the Claude's answer").is_empty());
        assert_eq!(
            learned(
                "le modèle d'antropic est bon",
                "le modèle d'Anthropic est bon"
            ),
            vec![pair("Anthropic", "antropic")]
        );
        assert_eq!(
            learned("c'est l’antropic", "c'est l’Anthropic"),
            vec![pair("Anthropic", "antropic")]
        );
        assert_eq!(
            learned(
                "D'antropic, je pense que oui",
                "D'Anthropic, je pense que oui"
            ),
            vec![pair("Anthropic", "antropic")]
        );
        assert_eq!(
            learned("L'antropic a publié", "L'Anthropic a publié"),
            vec![pair("Anthropic", "antropic")]
        );
        assert_eq!(
            learned("ask o'brian today", "ask O'Brien today"),
            vec![pair("O'Brien", "o'brian")]
        );
    }

    #[test]
    fn at_most_three_words_are_learned_from_one_dictation() {
        let inserted = "ask zorb quix vlem drub eli fay gus hal ivy jo kim lu mo about it";
        let edited = "ask Zarb Quax Vlam Drab eli fay gus hal ivy jo kim lu mo about it";
        assert_eq!(review(inserted, edited).len(), 3);
    }

    #[test]
    fn the_edited_span_is_found_between_untouched_text() {
        let before = "Dear team,\nI talked to Clawd today.\nThanks";
        let after = "Dear team,\nI talked to Claude today.\nThanks";
        assert_eq!(
            edited_span(before, "I talked to Clawd today.", after),
            Some("I talked to Claude today.")
        );
    }

    #[test]
    fn edits_outside_the_pasted_text_disable_learning() {
        let before = "Dear team,\nI talked to Clawd today.";
        assert_eq!(
            edited_span(
                before,
                "I talked to Clawd today.",
                "Hi all,\nI talked to Claude today."
            ),
            None
        );
        assert_eq!(
            edited_span("unrelated", "I talked to Clawd", "unrelated"),
            None
        );
        assert_eq!(edited_span("abc", "", "abc"), None);
    }

    #[test]
    fn the_diff_aligns_words_ignoring_punctuation() {
        assert_eq!(
            diff(&["a", "b,", "c"], &["a", "b", "x", "c"]),
            vec![Hunk {
                old: 2..2,
                new: 2..3
            }]
        );
        assert_eq!(
            diff(&["a", "b", "c"], &["a", "y", "c"]),
            vec![Hunk {
                old: 1..2,
                new: 1..2
            }]
        );
    }
}
