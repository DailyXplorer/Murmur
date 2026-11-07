//! The text of the focused field on each side of the insertion point, cut
//! to a short excerpt on word boundaries. Platform-free: the reads go
//! through `FocusedText`, which the Accessibility element implements, or
//! `SelectionText`, which the UI Automation element implements.

use std::fmt;
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;

use crate::leading_space::Before;

pub const MAX_BEFORE_CHARS: usize = 300;
pub const MAX_AFTER_CHARS: usize = 100;
// Two subdivision flags need 28 UTF-16 units; two four-person ZWJ families need 22.
const CARET_WINDOW_UNITS: usize = 32;

/// A selection or insertion point in UTF-16 code units, as Accessibility reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utf16Range {
    pub start: usize,
    pub len: usize,
}

/// Text of the focused field around the insertion point. The selection itself
/// is left out: the dictation replaces it.
#[derive(Clone, PartialEq, Eq)]
pub struct FieldExcerpt {
    pub before: String,
    pub after: String,
}

/// Lengths only, so a stray `{:?}` never logs what the user was writing.
impl fmt::Debug for FieldExcerpt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldExcerpt")
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .finish()
    }
}

/// Up to MAX_BEFORE_CHARS chars before the selection and MAX_AFTER_CHARS after it,
/// cut on word boundaries. None when both are blank.
pub fn cut(text: &str, selection: Utf16Range) -> Option<FieldExcerpt> {
    let start = byte_offset(text, selection.start);
    let end = byte_offset(text, selection.start.saturating_add(selection.len)).max(start);
    let before = last_words(&text[..start], MAX_BEFORE_CHARS);
    let after = first_words(&text[end..], MAX_AFTER_CHARS);
    (!before.trim().is_empty() || !after.trim().is_empty()).then(|| FieldExcerpt {
        before: before.to_string(),
        after: after.to_string(),
    })
}

/// The byte index of the char holding UTF-16 unit `offset`, so an offset
/// inside a surrogate pair rounds down; the end of `text` past it.
fn byte_offset(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > offset {
            return index;
        }
    }
    text.len()
}

/// The last `max` chars, without the word the limit cuts through.
fn last_words(text: &str, max: usize) -> &str {
    let count = text.chars().count();
    let Some((cut, _)) = text.char_indices().nth(count.saturating_sub(max)) else {
        return text;
    };
    if count <= max || text[..cut].ends_with(char::is_whitespace) {
        return &text[cut..];
    }
    let kept = &text[cut..];
    kept.find(char::is_whitespace)
        .map_or("", |space| kept[space..].trim_start())
}

/// The first `max` chars, without the word the limit cuts through.
fn first_words(text: &str, max: usize) -> &str {
    let Some((cut, _)) = text.char_indices().nth(max) else {
        return text;
    };
    let kept = &text[..cut];
    if text[cut..].starts_with(char::is_whitespace) {
        return kept;
    }
    kept.rfind(char::is_whitespace)
        .map_or("", |space| kept[..space].trim_end())
}

/// Read-only access to the focused element's text and selection.
#[cfg(any(target_os = "macos", test))]
pub trait FocusedText {
    fn is_secure(&self) -> bool;
    /// AXSelectedTextRange.
    fn selection(&self) -> Option<Utf16Range>;
    /// AXNumberOfCharacters, in UTF-16 units.
    fn char_count(&self) -> Option<usize>;
    /// AXStringForRange.
    fn string_for_range(&self, range: Utf16Range) -> Option<String>;
    /// AXValue.
    fn value(&self) -> Option<String>;
}

/// Grapheme context before a caret. Never reads secure text or guesses a caret
/// from a selection, and starts no read after `deadline`.
#[cfg(any(target_os = "macos", test))]
pub fn char_before(focused: &impl FocusedText, deadline: Instant) -> Option<Before> {
    let in_time = || Instant::now() < deadline;
    if !in_time() || focused.is_secure() || !in_time() {
        return None;
    }
    let selection = focused.selection()?;
    if selection.len != 0 || selection.start == 0 || !in_time() {
        return None;
    }
    let start = selection.start.saturating_sub(CARET_WINDOW_UNITS);
    let text = focused.string_for_range(Utf16Range {
        start,
        len: selection.start - start,
    })?;
    before_from_window(&text, start != 0)
}

/// The grapheme context at the end of `window`, the text just before a
/// caret. `cut_at_left` says the window may start inside a grapheme or an
/// elision's word, so the first grapheme is dropped and a missing
/// `previous` is no reason to guess.
fn before_from_window(window: &str, cut_at_left: bool) -> Option<Before> {
    let mut graphemes = window.graphemes(true);
    if cut_at_left {
        // The first grapheme may have lost its base at the window's left edge.
        let _ = graphemes.next();
    }
    let last = graphemes.next_back()?;
    let previous = graphemes.next_back();
    // A cut elision letter must not turn its apostrophe into a field-start quote.
    if cut_at_left && previous.is_none() {
        return None;
    }
    Before::from_graphemes(last, previous)
}

/// The excerpt around the focused element's selection. Never reads a secure
/// field, and never guesses the cursor: no selection range, no excerpt.
/// Each read may take a hung app's whole messaging timeout, so none starts
/// after `deadline`.
#[cfg(any(target_os = "macos", test))]
pub fn read_field(focused: &impl FocusedText, deadline: Instant) -> Option<FieldExcerpt> {
    let in_time = || Instant::now() < deadline;
    if focused.is_secure() || !in_time() {
        return None;
    }
    let selection = focused.selection()?;
    let (text, offset) = match window_around(focused, selection, in_time) {
        Some(window) => window,
        None if in_time() => (focused.value()?, 0),
        None => return None,
    };
    cut(
        &text,
        Utf16Range {
            start: selection.start.saturating_sub(offset),
            len: selection.len,
        },
    )
}

/// Just enough of the field for `cut`, and where it starts. A UTF-16 unit
/// is at least half a char, so a window that does not start at the field's
/// start holds more than the limit and `cut` drops its partial first word.
#[cfg(any(target_os = "macos", test))]
fn window_around(
    focused: &impl FocusedText,
    selection: Utf16Range,
    in_time: impl Fn() -> bool,
) -> Option<(String, usize)> {
    if !in_time() {
        return None;
    }
    let count = focused.char_count()?;
    let selection_end = selection.start.saturating_add(selection.len);
    let end = selection_end
        .saturating_add(2 * MAX_AFTER_CHARS + 2)
        .min(count);
    let start = selection
        .start
        .saturating_sub(2 * MAX_BEFORE_CHARS + 2)
        .min(end);
    if !in_time() {
        return None;
    }
    let text = focused.string_for_range(Utf16Range {
        start,
        len: end - start,
    })?;
    Some((text, start))
}

/// The reads `read_around_selection` needs from a focused field that gives
/// the text next to its selection but not the selection's offset, as UI
/// Automation's text ranges do.
#[cfg(any(target_os = "windows", test))]
pub trait SelectionText {
    type Selection;
    fn is_secure(&self) -> bool;
    /// The selection, or the insertion point as an empty selection.
    fn selection(&self) -> Option<Self::Selection>;
    /// Whether the selection is an insertion point, with no text selected.
    fn is_caret(&self, selection: &Self::Selection) -> Option<bool>;
    /// At most `max` chars just before the selection.
    fn before(&self, selection: &Self::Selection, max: usize) -> Option<String>;
    /// At most `max` chars just after the selection.
    fn after(&self, selection: &Self::Selection, max: usize) -> Option<String>;
}

/// Grapheme context before a caret, as `char_before` gives it, for a field
/// that gives the text next to its selection. Never reads secure text, a
/// non-empty selection or a guessed caret, and starts no read after
/// `deadline`. A read that fills the window may have been cut at its left
/// edge, as `char_before`'s does when it starts inside the field.
#[cfg(any(target_os = "windows", test))]
pub fn char_before_selection(focused: &impl SelectionText, deadline: Instant) -> Option<Before> {
    let in_time = || Instant::now() < deadline;
    if !in_time() || focused.is_secure() || !in_time() {
        return None;
    }
    let selection = focused.selection()?;
    if !in_time() || !focused.is_caret(&selection)? || !in_time() {
        return None;
    }
    let window = focused.before(&selection, CARET_WINDOW_UNITS)?;
    before_from_window(&window, window.encode_utf16().count() >= CARET_WINDOW_UNITS)
}

/// The excerpt around the focused element's selection, as `read_field`
/// gives it: never from a secure field, never without a selection or
/// insertion point, and no read starts after `deadline`. Each side is read
/// with twice its limit, so `cut` drops a word the read cut through.
#[cfg(any(target_os = "windows", test))]
pub fn read_around_selection(
    focused: &impl SelectionText,
    deadline: Instant,
) -> Option<FieldExcerpt> {
    let in_time = || Instant::now() < deadline;
    if focused.is_secure() || !in_time() {
        return None;
    }
    let selection = focused.selection()?;
    let before = in_time()
        .then(|| focused.before(&selection, 2 * MAX_BEFORE_CHARS + 2))
        .flatten()?;
    let after = in_time()
        .then(|| focused.after(&selection, 2 * MAX_AFTER_CHARS + 2))
        .flatten()?;
    let start = before.encode_utf16().count();
    cut(&(before + &after), Utf16Range { start, len: 0 })
}

#[cfg(test)]
mod tests;
