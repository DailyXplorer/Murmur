//! The user's dictionary. Every word lives in `custom_words`, which
//! transcription already applies; `learned_words` marks the ones Murmur
//! learned from a correction and remembers how they were misheard.

mod learn;
mod observer;

pub use observer::{stop_observing, watch_pasted_text};

use crate::audio_toolkit::apply_custom_words_counted;
use crate::insights::InsightsManager;
use crate::settings::{self, MAX_CUSTOM_WORDS};
use learn::Correction;
use log::{debug, error, info};
use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
pub struct LearnedWord {
    pub word: String,
    /// How Murmur wrote it before the user corrected it.
    pub heard_as: String,
}

/// Drops learned entries whose word was removed from the dictionary, and
/// duplicates. Returns whether anything changed.
pub fn prune_learned(learned: &mut Vec<LearnedWord>, custom_words: &[String]) -> bool {
    let before = learned.len();
    let mut seen = HashSet::new();
    learned.retain(|entry| {
        custom_words.contains(&entry.word)
            && !entry.heard_as.trim().is_empty()
            && seen.insert(entry.word.clone())
    });
    learned.len() != before
}

/// Applies the dictionary to a transcript. Words the user typed in are
/// fuzzy matched as before. A learned word is replaced only where its exact
/// misheard form appears: fuzzy matching it would turn "cloud" into "Claude"
/// from a single correction, so otherwise it is only a vocabulary hint for
/// cleanup. Also returns how many replacements were made, for the Insights
/// "dictionary fixes" count: each replaced span counts once, since a word
/// a learned entry replaced is not matched again against the typed words.
pub fn correct(
    text: &str,
    custom_words: &[String],
    learned: &[LearnedWord],
    threshold: f64,
) -> (String, u32) {
    let typed: Vec<String> = custom_words
        .iter()
        .filter(|word| !learned.iter().any(|entry| &entry.word == *word))
        .cloned()
        .collect();
    let (heard, learned_fixes) = apply_heard_as(text, learned);
    let (corrected, typed_fixes) = apply_custom_words_counted(&heard, &typed, threshold);
    (corrected, learned_fixes + typed_fixes)
}

/// Replaces each learned mishearing with its word, whole words only and
/// ignoring case. Entries whose misheard form is also said on purpose, such
/// as an everyday word or a near-identical ordinary name, stay hints only;
/// this also covers entries stored before those rules existed. Returns the
/// text and the number of replacements.
fn apply_heard_as(text: &str, learned: &[LearnedWord]) -> (String, u32) {
    let safe = learned
        .iter()
        .filter(|entry| learn::replaces_safely(&entry.heard_as, &entry.word));
    safe.fold((text.to_string(), 0), |(text, fixes), entry| {
        let pattern = format!(r"(?i)\b{}\b", regex::escape(entry.heard_as.trim()));
        let Ok(heard) = Regex::new(&pattern) else {
            return (text, fixes);
        };
        let found = heard
            .find_iter(&text)
            .filter(|found| found.as_str() != entry.word)
            .count() as u32;
        if found == 0 {
            return (text, fixes);
        }
        let replaced = heard
            .replace_all(&text, regex::NoExpand(&entry.word))
            .into_owned();
        (replaced, fixes + found)
    })
}

/// Adds learned words to the dictionary and tells the settings UI.
fn remember(app: &AppHandle, corrections: Vec<Correction>) {
    let mut value = settings::get_settings(app);
    let mut added = 0;
    for correction in corrections {
        if value.custom_words.contains(&correction.word) {
            continue;
        }
        if value.custom_words.len() >= MAX_CUSTOM_WORDS {
            break;
        }
        value.custom_words.push(correction.word.clone());
        value.learned_words.push(LearnedWord {
            word: correction.word,
            heard_as: correction.heard_as,
        });
        added += 1;
    }
    if added == 0 {
        return;
    }
    settings::write_settings(app, value);
    info!("Learned {added} dictionary word(s) from a correction");
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({"setting": "custom_words"}),
    );
}

/// Records in Insights that the user edited a pasted dictation, once per
/// dictation, with the number of words changed. Runs on the watcher thread.
fn corrections_observed(app: &AppHandle, words_corrected: u32) {
    debug!("User corrected {words_corrected} pasted word(s)");
    let Some(insights) = app.try_state::<Arc<InsightsManager>>() else {
        return;
    };
    if let Err(e) = insights.record_correction(words_corrected) {
        error!("Failed to record corrected words in insights: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn learned(word: &str, heard_as: &str) -> LearnedWord {
        LearnedWord {
            word: word.to_string(),
            heard_as: heard_as.to_string(),
        }
    }

    #[test]
    fn heard_as_is_replaced_as_whole_words_in_any_case() {
        let entries = [
            learned("Claude", "clawd"),
            learned("Kubernetes", "cooper netties"),
        ];
        assert_eq!(
            apply_heard_as("Clawd runs on Cooper Netties, not clawdia", &entries),
            ("Claude runs on Kubernetes, not clawdia".to_string(), 2)
        );
    }

    #[test]
    fn a_stored_common_word_mishearing_is_never_replaced() {
        let entries = [learned("Claude", "cloud"), learned("Sarah's", "Sarah")];
        assert_eq!(
            apply_heard_as("cloud storage for Sarah", &entries),
            ("cloud storage for Sarah".to_string(), 0)
        );
    }

    #[test]
    fn a_learned_word_is_not_fuzzy_matched_onto_ordinary_words() {
        let words = vec!["Claude".to_string(), "ChatGPT".to_string()];
        let entries = [learned("Claude", "clawd")];
        assert_eq!(
            correct("ask clawd about cloud storage", &words, &entries, 0.18),
            ("ask Claude about cloud storage".to_string(), 1)
        );
        assert_eq!(
            correct("open chat gpt", &words, &entries, 0.18),
            ("open ChatGPT".to_string(), 1)
        );
    }

    #[test]
    fn a_near_identical_ordinary_name_stays_a_vocabulary_hint() {
        let words = vec!["Smyth".to_string()];
        let entries = [learned("Smyth", "Smith")];
        assert_eq!(
            correct("call Smith about the Smith account", &words, &entries, 0.18),
            ("call Smith about the Smith account".to_string(), 0)
        );
        assert_eq!(
            apply_heard_as("Clawd says hi", &[learned("Claude", "Clawd")]),
            ("Claude says hi".to_string(), 1)
        );
    }

    #[test]
    fn a_joined_pair_of_common_words_is_replaced_as_a_sequence() {
        let entries = [learned("OpenAI", "open ai")];
        assert_eq!(
            apply_heard_as("ask open ai, then open the AI panel", &entries),
            ("ask OpenAI, then open the AI panel".to_string(), 1)
        );
    }

    #[test]
    fn replacement_text_is_literal() {
        assert_eq!(
            apply_heard_as("pay dollar sign", &[learned("$1", "dollar sign")]),
            ("pay $1".to_string(), 1)
        );
    }

    #[test]
    fn each_dictionary_fix_counts_once() {
        let words = vec!["Claude".to_string(), "ChargeBee".to_string()];
        let entries = [learned("Claude", "clawd")];
        assert_eq!(
            correct(
                "clawd and Clawd use Charge B, not Claude",
                &words,
                &entries,
                0.18
            ),
            ("Claude and Claude use ChargeBee, not Claude".to_string(), 3)
        );
    }

    #[test]
    fn learned_entries_follow_the_dictionary() {
        let mut entries = vec![
            learned("Claude", "clawd"),
            learned("Removed", "removd"),
            learned("Claude", "cloud"),
            learned("Empty", " "),
        ];
        let words = vec!["Claude".to_string(), "Empty".to_string()];
        assert!(prune_learned(&mut entries, &words));
        assert_eq!(entries, vec![learned("Claude", "clawd")]);
        assert!(!prune_learned(&mut entries, &words));
    }
}
