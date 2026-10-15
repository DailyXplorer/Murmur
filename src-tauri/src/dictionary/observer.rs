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
        let mut latest = before.clone();
        while started.elapsed() < WATCH_WINDOW {
            std::thread::sleep(POLL_INTERVAL);
            if GENERATION.load(Ordering::Acquire) != generation {
                break;
            }
            let still_focused = focused_text_element().is_some_and(|focused| focused == field);
            match read(&field) {
                Some(value) => latest = value,
                None => break,
            }
            if !still_focused {
                break;
            }
        }

        let Some(edited) = learn::edited_span(&before, &inserted, &latest) else {
            return;
        };
        let review = learn::review(inserted.trim(), edited);
        if review.words_corrected > 0 {
            super::corrections_observed(&app, review.words_corrected);
        }
        if !review.learned.is_empty() {
            super::remember(&app, review.learned);
        }
    });
}

fn read(field: &Element) -> Option<String> {
    if field.is_secure() {
        return None;
    }
    field
        .string("AXValue")
        .filter(|value| value.chars().count() <= MAX_FIELD_CHARS)
}
