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
const CONFIRM_DELAY: Duration = Duration::from_millis(200);
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
        let snapshots = snapshots(before.clone(), |delay| {
            if !still_focused || started.elapsed() >= WATCH_WINDOW {
                return None;
            }
            std::thread::sleep(delay);
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

/// The field's text after each wait. `wait_and_read` sleeps for the delay it
/// is given, then reads the field, or returns `None` to end the watch. A
/// changed field is read again after `CONFIRM_DELAY`, so a correction sent
/// right after it was typed still gets the second read that settles it.
fn snapshots(
    before: String,
    mut wait_and_read: impl FnMut(Duration) -> Option<String>,
) -> impl Iterator<Item = String> {
    let mut last = before;
    let mut changed = false;
    std::iter::from_fn(move || {
        let delay = if changed {
            CONFIRM_DELAY
        } else {
            POLL_INTERVAL
        };
        let snapshot = wait_and_read(delay)?;
        changed = snapshot != last;
        last.clone_from(&snapshot);
        Some(snapshot)
    })
}

/// What the user left in place of `inserted`: the last span two consecutive
/// snapshots agree on, so a word caught halfway through typing is never
/// learned. Chat apps empty the field when the message is sent, so the
/// first snapshot where the span is gone or blank ends the watch.
fn settled_edit(
    before: &str,
    inserted: &str,
    snapshots: impl IntoIterator<Item = String>,
) -> Option<String> {
    let mut stable = learn::edited_span(before, inserted, before)?.to_string();
    let mut previous: Option<String> = None;
    for snapshot in snapshots {
        let span = match learn::edited_span(before, inserted, &snapshot) {
            Some(span) if !span.trim().is_empty() => span.to_string(),
            _ => break,
        };
        if previous.as_ref() == Some(&span) {
            stable.clone_from(&span);
        }
        previous = Some(span);
    }
    Some(stable)
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
        let corrected = "I talked to Claude about it";
        let expected = vec![("Claude".to_string(), "clawd".to_string())];
        assert_eq!(
            learned(inserted, inserted, &[corrected, corrected, ""]),
            expected
        );
        assert_eq!(
            learned(inserted, inserted, &[corrected, corrected, "", "ok thanks"]),
            expected
        );
        let greeted = "Hi, I talked to Claude about it";
        assert_eq!(
            learned(
                "Hi, I talked to clawd about it",
                inserted,
                &[greeted, greeted, "Hi, "]
            ),
            expected
        );
    }

    #[test]
    fn a_word_caught_halfway_through_typing_is_not_learned() {
        let inserted = "I talked to clawd about it";
        assert!(learned(inserted, inserted, &["I talked to Claud about it", ""]).is_empty());
        assert_eq!(
            learned(
                inserted,
                inserted,
                &[
                    "I talked to Claude about it",
                    "I talked to Claude about it",
                    "I talked to Claud about it",
                    ""
                ]
            ),
            vec![("Claude".to_string(), "clawd".to_string())]
        );
    }

    #[test]
    fn a_correction_in_a_field_that_keeps_its_text_is_learned() {
        let inserted = "I talked to clawd about it";
        let corrected = "I talked to Claude about it";
        assert_eq!(
            learned(
                inserted,
                inserted,
                &["I talked to cl about it", corrected, corrected]
            ),
            vec![("Claude".to_string(), "clawd".to_string())]
        );
    }

    /// What the watch learns when the field shows `before` until the first
    /// timed text, each text holding from its millisecond until the next.
    fn learned_over_time(before: &str, timeline: &[(u64, &str)]) -> Vec<(String, String)> {
        let mut now = Duration::ZERO;
        let field = snapshots(before.to_string(), |delay| {
            now += delay;
            if now >= WATCH_WINDOW {
                return None;
            }
            let text = timeline
                .iter()
                .rev()
                .find(|(at, _)| Duration::from_millis(*at) <= now)
                .map_or(before, |(_, text)| text);
            Some(text.to_string())
        });
        let edited = settled_edit(before, before, field).unwrap();
        learn::review(before, &edited)
            .into_iter()
            .map(|correction| (correction.word, correction.heard_as))
            .collect()
    }

    #[test]
    fn a_correction_sent_right_after_the_edit_is_learned() {
        let inserted = "I talked to clawd about it";
        let corrected = "I talked to Claude about it";
        let expected = vec![("Claude".to_string(), "clawd".to_string())];
        // Polls at 0.75, 1.5, 2.25 and 3 s. Enter at 3.3 s empties the field.
        assert_eq!(
            learned_over_time(inserted, &[(2_900, corrected), (3_300, "")]),
            expected
        );
        assert_eq!(
            learned_over_time(
                inserted,
                &[
                    (2_200, "I talked to Cl about it"),
                    (2_400, corrected),
                    (2_700, "")
                ]
            ),
            expected
        );
    }

    #[test]
    fn a_word_sent_halfway_through_typing_is_not_learned() {
        let inserted = "I talked to clawd about it";
        assert!(learned_over_time(
            inserted,
            &[(2_900, "I talked to Claud about it"), (3_050, "")]
        )
        .is_empty());
    }

    #[test]
    fn a_field_emptied_before_any_edit_teaches_nothing() {
        let inserted = "I talked to clawd about it";
        assert!(learned(inserted, inserted, &["", "I talked to Claude about it"]).is_empty());
    }
}
