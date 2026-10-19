//! The user's dictionary. Every word lives in `custom_words` and is sent to
//! cleanup as vocabulary. The words the user typed in also replace their
//! fuzzy matches in the transcript. `learned_words` marks the ones Murmur
//! learned from a correction: those are hints only and never rewrite text.

mod learn;
mod observer;

pub(crate) use learn::{is_common_word, ELISIONS};
pub use observer::{stop_observing, watch_pasted_text};

use crate::audio_toolkit::apply_custom_words_counted;
use crate::insights::InsightsManager;
use crate::settings::{self, MAX_CUSTOM_WORDS};
use learn::Correction;
use log::{error, info};
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

/// Applies the words the user typed into the dictionary to a transcript,
/// fuzzy matched. Learned words are left out: like Wispr Flow's learned
/// names and VoiceInk's vocabulary, they are only hints for cleanup, so a
/// learned "Claude" never turns a spoken "cloud" into a name. Also returns
/// how many replacements were made, for the Insights "dictionary fixes".
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
    apply_custom_words_counted(text, &typed, threshold)
}

/// Adds learned words to the dictionary, tells the settings UI, and counts
/// each new word once in Insights as a word the user corrected.
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
    let Some(insights) = app.try_state::<Arc<InsightsManager>>() else {
        return;
    };
    if let Err(e) = insights.record_correction(added) {
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
    fn a_learned_word_never_rewrites_the_transcript() {
        let words = vec!["Claude".to_string(), "Q4".to_string(), "Smyth".to_string()];
        let entries = [
            learned("Claude", "clawd"),
            learned("Q4", "Q3"),
            learned("Smyth", "Smith"),
        ];
        assert_eq!(
            correct(
                "ask clawd about cloud storage, Q3 numbers and Smith",
                &words,
                &entries,
                0.18
            ),
            (
                "ask clawd about cloud storage, Q3 numbers and Smith".to_string(),
                0
            )
        );
    }

    #[test]
    fn words_the_user_typed_still_replace_their_matches() {
        let words = vec![
            "Claude".to_string(),
            "ChatGPT".to_string(),
            "ChargeBee".to_string(),
        ];
        let entries = [learned("Claude", "clawd")];
        assert_eq!(
            correct(
                "open chat gpt and Charge B with clawd",
                &words,
                &entries,
                0.18
            ),
            ("open ChatGPT and ChargeBee with clawd".to_string(), 2)
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
