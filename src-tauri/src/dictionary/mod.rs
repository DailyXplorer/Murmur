//! The user's dictionary. Every word lives in `custom_words`, which
//! transcription already applies; `learned_words` marks the ones Murmur
//! learned from a correction and remembers how they were misheard.

mod learn;
mod observer;

pub use observer::{stop_observing, watch_pasted_text};

use crate::audio_toolkit::apply_custom_words;
use crate::settings::{self, MAX_CUSTOM_WORDS};
use learn::Correction;
use log::{debug, info};
use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use tauri::{AppHandle, Emitter};

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
/// cleanup.
pub fn correct(
    text: &str,
    custom_words: &[String],
    learned: &[LearnedWord],
    threshold: f64,
) -> String {
    let typed: Vec<String> = custom_words
        .iter()
        .filter(|word| !learned.iter().any(|entry| &entry.word == *word))
        .cloned()
        .collect();
    apply_custom_words(&apply_heard_as(text, learned), &typed, threshold)
}

/// Replaces each learned mishearing with its word, whole words only and
/// ignoring case. Entries stored before a mishearing had to be an unusual
/// form are skipped.
fn apply_heard_as(text: &str, learned: &[LearnedWord]) -> String {
    let safe = learned
        .iter()
        .filter(|entry| learn::is_safe_mishearing(&entry.heard_as));
    safe.fold(text.to_string(), |text, entry| {
        let pattern = format!(r"(?i)\b{}\b", regex::escape(entry.heard_as.trim()));
        match Regex::new(&pattern) {
            Ok(heard) => heard
                .replace_all(&text, regex::NoExpand(&entry.word))
                .into_owned(),
            Err(_) => text,
        }
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

/// Hook for the insights record of corrections: called once per dictation
/// the user edited after it was pasted, with the number of words changed.
pub fn corrections_observed(_app: &AppHandle, words_corrected: u32) {
    debug!("User corrected {words_corrected} pasted word(s)");
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
            "Claude runs on Kubernetes, not clawdia"
        );
    }

    #[test]
    fn a_stored_common_word_mishearing_is_never_replaced() {
        let entries = [learned("Claude", "cloud"), learned("Sarah's", "Sarah")];
        assert_eq!(
            apply_heard_as("cloud storage for Sarah", &entries),
            "cloud storage for Sarah"
        );
    }

    #[test]
    fn a_learned_word_is_not_fuzzy_matched_onto_ordinary_words() {
        let words = vec!["Claude".to_string(), "ChatGPT".to_string()];
        let entries = [learned("Claude", "clawd")];
        assert_eq!(
            correct("ask clawd about cloud storage", &words, &entries, 0.18),
            "ask Claude about cloud storage"
        );
        assert_eq!(
            correct("open chat gpt", &words, &entries, 0.18),
            "open ChatGPT"
        );
    }

    #[test]
    fn replacement_text_is_literal() {
        assert_eq!(
            apply_heard_as("pay dollar sign", &[learned("$1", "dollar sign")]),
            "pay $1"
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
