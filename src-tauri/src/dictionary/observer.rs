//! Watches the field Murmur just pasted into, so a correction the user makes
//! there can teach the dictionary. The field's text stays in this thread's
//! memory: it is never stored or logged, and only the corrected words leave.

use super::learn;
use crate::accessibility::{focused_text_element, Element};
use log::info;
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
    std::thread::spawn(move || match watch(&inserted, generation) {
        Ok(learned) => super::remember(&app, learned),
        Err(stop) => info!("Correction learning: nothing learned, {stop}"),
    });
}

/// Why a watch ended without learning. Counts only: the field's text and the
/// words never reach the log.
#[derive(Debug, PartialEq)]
enum Stop {
    NoField,
    PastedTextNotFound,
    FocusMoved { polls: u32 },
    NoSettledEdit { polls: u32 },
    Rejected(learn::Rejected),
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoField => write!(f, "no readable text field was focused"),
            Self::PastedTextNotFound => write!(f, "the pasted text was not found in the field"),
            Self::FocusMoved { polls } => {
                write!(f, "focus moved before an edit settled (polls: {polls})")
            }
            Self::NoSettledEdit { polls } => write!(f, "no settled edit (polls: {polls})"),
            Self::Rejected(rejected) => rejected.fmt(f),
        }
    }
}

fn watch(inserted: &str, generation: u64) -> Result<Vec<learn::Correction>, Stop> {
    std::thread::sleep(SETTLE_DELAY);
    let field = focused_text_element().ok_or(Stop::NoField)?;
    let before = read(&field).ok_or(Stop::NoField)?;
    if !before.contains(inserted.trim()) {
        return Err(Stop::PastedTextNotFound);
    }

    let started = Instant::now();
    let mut still_focused = true;
    let mut polls = 0;
    let snapshots = std::iter::from_fn(|| {
        if !still_focused || started.elapsed() >= WATCH_WINDOW {
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
        if GENERATION.load(Ordering::Acquire) != generation {
            return None;
        }
        polls += 1;
        still_focused = focused_text_element().is_some_and(|focused| focused == field);
        read(&field)
    });

    let edited = settled_edit(&before, inserted, snapshots).ok_or(Stop::PastedTextNotFound)?;
    review(inserted, &edited, still_focused, polls)
}

/// Reviews the settled edit, or says why there was none: an untouched span
/// means the user never fixed anything before the watch ended.
fn review(
    inserted: &str,
    edited: &str,
    still_focused: bool,
    polls: u32,
) -> Result<Vec<learn::Correction>, Stop> {
    let inserted = inserted.trim();
    if edited == inserted {
        return Err(if still_focused {
            Stop::NoSettledEdit { polls }
        } else {
            Stop::FocusMoved { polls }
        });
    }
    learn::review(inserted, edited).map_err(Stop::Rejected)
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
            .unwrap_or_default()
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

    #[test]
    fn a_field_emptied_before_any_edit_teaches_nothing() {
        let inserted = "I talked to clawd about it";
        assert!(learned(inserted, inserted, &["", "I talked to Claude about it"]).is_empty());
    }

    #[test]
    fn a_watch_without_a_fix_says_whether_focus_moved() {
        let inserted = " I talked to clawd about it ";
        let untouched = inserted.trim();
        assert_eq!(
            review(inserted, untouched, true, 60),
            Err(Stop::NoSettledEdit { polls: 60 })
        );
        assert_eq!(
            review(inserted, untouched, false, 2),
            Err(Stop::FocusMoved { polls: 2 })
        );
        assert_eq!(
            review(inserted, "I talked to Clawd about it", false, 2),
            Err(Stop::Rejected(learn::Rejected::NoCandidate { changes: 0 }))
        );
        assert_eq!(
            review(inserted, "I talked to Claude about it", false, 2).map(|words| words.len()),
            Ok(1)
        );
        assert_eq!(
            Stop::FocusMoved { polls: 2 }.to_string(),
            "focus moved before an edit settled (polls: 2)"
        );
    }
}
