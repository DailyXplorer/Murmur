//! The user's dictionary. Every word lives in `custom_words` and is sent to
//! cleanup as vocabulary. The words the user typed in also replace their
//! fuzzy matches in the transcript. `learned_words` marks the ones Murmur
//! learned from a correction: those are hints only and never rewrite text.

// Learning reads the field the user corrects through Accessibility, which
// has no Windows counterpart here: Windows only uses the word lists.
#[cfg_attr(target_os = "windows", allow(dead_code))]
mod learn;
#[cfg(target_os = "macos")]
mod observer;

pub(crate) use learn::{is_everyday_word, ELISIONS};
#[cfg(target_os = "macos")]
pub use observer::{stop_observing, watch_pasted_text};

use crate::audio_toolkit::apply_custom_words_counted;
#[cfg(target_os = "macos")]
use {
    crate::insights::InsightsManager,
    crate::settings::{self, AppSettings, MAX_CUSTOM_WORDS},
    learn::Correction,
    log::{error, info},
    std::sync::Arc,
    tauri::{AppHandle, Emitter, Manager},
};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;

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
#[cfg(target_os = "macos")]
fn remember(app: &AppHandle, corrections: Vec<Correction>) {
    let added = settings::update_settings(app, |value| remember_corrections(value, corrections));
    if added == 0 {
        return;
    }
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

// A pure field-owned mutation, shared by the watcher and regression tests.
#[cfg(target_os = "macos")]
fn remember_corrections(value: &mut AppSettings, corrections: Vec<Correction>) -> u32 {
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
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    fn correction(word: &str) -> Correction {
        Correction {
            word: word.to_string(),
            heard_as: "misheard".to_string(),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn learning_preserves_other_preferences_and_only_counts_new_words() {
        let mut value = settings::get_default_settings();
        value.window_names_enabled = false;
        value.custom_words.push("Existing".to_string());
        let mut expected = serde_json::to_value(&value).unwrap();
        assert_eq!(
            remember_corrections(
                &mut value,
                vec![correction("Existing"), correction("Murmur")]
            ),
            1
        );
        expected["custom_words"] = serde_json::json!(["Existing", "Murmur"]);
        expected["learned_words"] = serde_json::json!([{"word": "Murmur", "heard_as": "misheard"}]);
        assert_eq!(serde_json::to_value(&value).unwrap(), expected);
        assert_eq!(
            remember_corrections(&mut value, vec![correction("Murmur")]),
            0
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn learning_checks_the_current_dictionary_capacity() {
        let mut value = settings::get_default_settings();
        value.custom_words = (0..MAX_CUSTOM_WORDS - 1)
            .map(|i| format!("Word{i}"))
            .collect();
        assert_eq!(
            remember_corrections(&mut value, vec![correction("First"), correction("Second")]),
            1
        );
        assert_eq!(value.custom_words.len(), MAX_CUSTOM_WORDS);
        assert_eq!(value.learned_words, vec![learned("First", "misheard")]);
        assert_eq!(
            remember_corrections(&mut value, vec![correction("Second")]),
            0
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn concurrent_learning_and_context_change_preserve_both_settings() {
        use std::sync::{mpsc, Mutex};
        use std::thread;
        use std::time::Duration;

        let transactions = Arc::new(settings::Transactions::new());
        let store = Arc::new(Mutex::new(settings::get_default_settings()));
        let (loaded_tx, loaded_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let learner = {
            let transactions = Arc::clone(&transactions);
            let store = Arc::clone(&store);
            thread::spawn(move || {
                transactions.update(
                    || {
                        assert!(transactions.is_locked());
                        Ok::<_, ()>(store.lock().unwrap().clone())
                    },
                    |value| {
                        loaded_tx.send(()).unwrap();
                        resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(remember_corrections(value, vec![correction("Murmur")]))
                    },
                    |value| {
                        *store.lock().unwrap() = value;
                        Ok(())
                    },
                )
            })
        };
        loaded_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            transactions.is_locked(),
            "the load and dictionary change must share the commit lock"
        );
        let toggle = {
            let transactions = Arc::clone(&transactions);
            let store = Arc::clone(&store);
            thread::spawn(move || {
                transactions.update(
                    || Ok::<_, ()>(store.lock().unwrap().clone()),
                    |value| {
                        value.window_names_enabled = false;
                        Ok(())
                    },
                    |value| {
                        *store.lock().unwrap() = value;
                        Ok(())
                    },
                )
            })
        };
        resume_tx.send(()).unwrap();
        assert_eq!(learner.join().unwrap(), Ok(1));
        toggle.join().unwrap().unwrap();
        let value = store.lock().unwrap();
        assert!(!value.window_names_enabled);
        assert_eq!(value.custom_words, ["Murmur"]);
        assert_eq!(value.learned_words, [learned("Murmur", "misheard")]);
    }

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
