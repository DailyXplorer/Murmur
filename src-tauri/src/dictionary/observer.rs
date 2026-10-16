//! Watches the field Murmur just pasted into, so a correction the user makes
//! there can teach the dictionary. The field's text stays in this thread's
//! memory: it is never stored or logged, and only the corrected words leave.

use super::learn;
use crate::accessibility::{focused_text_element, Element};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::AppHandle;

const SETTLE_DELAY: Duration = Duration::from_millis(300);
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const WATCH_WINDOW: Duration = Duration::from_secs(45);
/// Larger fields (whole documents) are skipped: diffing them every poll is
/// wasted work for a few edited words.
const MAX_FIELD_CHARS: usize = 100_000;

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Ends the current watch. The next dictation calls this before it starts,
/// so its own paste is never mistaken for a correction.
pub fn stop_observing() {
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Starts watching the focused field after `inserted` was pasted into it.
pub fn watch_pasted_text(app: &AppHandle, inserted: String) {
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SETTLE_DELAY);
        let Some(field) = focused_text_element() else {
            return;
        };
        let Some(before) = read(&field) else {
            return;
        };
        if !before.contains(inserted.trim()) {
            return;
        }

        let started = Instant::now();
        let mut still_focused = true;
        let snapshots = std::iter::from_fn(|| {
            if !still_focused || started.elapsed() >= WATCH_WINDOW {
                return None;
            }
            std::thread::sleep(POLL_INTERVAL);
            if GENERATION.load(Ordering::Acquire) != generation {
                return None;
            }
            still_focused = focused_text_element().is_some_and(|focused| focused == field);
            read(&field)
        });

        let Some(edited) = settled_edit(&before, &inserted, snapshots) else {
            return;
        };
        let learned = learn::review(inserted.trim(), &edited);
        if !learned.is_empty() {
            super::remember(&app, learned);
        }
    });
}

/// What the user left in place of `inserted`, as of the last snapshot that
/// still holds it. Chat apps empty the field when the message is sent, so
/// the first snapshot where the span is gone or blank ends the watch and
/// keeps the edit made before it.
fn settled_edit(
    before: &str,
    inserted: &str,
    snapshots: impl IntoIterator<Item = String>,
) -> Option<String> {
    let mut edited = learn::edited_span(before, inserted, before)?.to_string();
    for snapshot in snapshots {
        match learn::edited_span(before, inserted, &snapshot) {
            Some(span) if !span.trim().is_empty() => edited = span.to_string(),
            _ => break,
        }
    }
    Some(edited)
}

fn read(field: &Element) -> Option<String> {
    if field.is_secure() {
        return None;
    }
    field
        .string("AXValue")
        .filter(|value| value.chars().count() <= MAX_FIELD_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn learned(before: &str, inserted: &str, snapshots: &[&str]) -> Vec<(String, String)> {
        let snapshots = snapshots.iter().map(|snapshot| snapshot.to_string());
        let edited = settled_edit(before, inserted, snapshots).unwrap();
        learn::review(inserted.trim(), &edited)
            .into_iter()
            .map(|correction| (correction.word, correction.heard_as))
            .collect()
    }

    #[test]
    fn a_correction_is_learned_when_the_chat_message_is_sent() {
        let inserted = "I talked to clawd about it";
        let expected = vec![("Claude".to_string(), "clawd".to_string())];
        assert_eq!(
            learned(inserted, inserted, &["I talked to Claude about it", ""]),
            expected
        );
        assert_eq!(
            learned(
                inserted,
                inserted,
                &["I talked to Claude about it", "", "ok thanks"]
            ),
            expected
        );
        assert_eq!(
            learned(
                "Hi, I talked to clawd about it",
                inserted,
                &["Hi, I talked to Claude about it", "Hi, "]
            ),
            expected
        );
    }

    #[test]
    fn a_correction_in_a_field_that_keeps_its_text_is_learned() {
        let inserted = "I talked to clawd about it";
        assert_eq!(
            learned(
                inserted,
                inserted,
                &["I talked to cl about it", "I talked to Claude about it"]
            ),
            vec![("Claude".to_string(), "clawd".to_string())]
        );
    }

    #[test]
    fn a_field_emptied_before_any_edit_teaches_nothing() {
        let inserted = "I talked to clawd about it";
        assert!(learned(inserted, inserted, &["", "I talked to Claude about it"]).is_empty());
    }
}
