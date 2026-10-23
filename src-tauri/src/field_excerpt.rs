//! The text of the focused field on each side of the insertion point, cut
//! to a short excerpt on word boundaries. Platform-free: the reads go
//! through `FocusedText`, which the Accessibility element implements.

use std::fmt;
use std::time::Instant;

pub const MAX_BEFORE_CHARS: usize = 300;
pub const MAX_AFTER_CHARS: usize = 100;
/// How far before the insertion point the field is read and checked, for
/// the label of a long list above the excerpt.
const CHECK_BEFORE_UNITS: usize = 2_000;

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

/// The reads `read_field` needs from the focused element.
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

/// The excerpt around the focused element's selection. Never reads a secure
/// field, and never guesses the cursor: no selection range, no excerpt.
/// Each read may take a hung app's whole messaging timeout, so none starts
/// after `deadline`. None when `withheld` holds for all the text read, up
/// to `CHECK_BEFORE_UNITS` before the cursor: a label the cut leaves out
/// still marks the values it keeps, however its list is laid out.
pub fn read_field(
    focused: &impl FocusedText,
    deadline: Instant,
    withheld: impl Fn(&str) -> bool,
) -> Option<FieldExcerpt> {
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
    if withheld(&text) {
        return None;
    }
    cut(
        &text,
        Utf16Range {
            start: selection.start.saturating_sub(offset),
            len: selection.len,
        },
    )
}

/// Enough of the field for `cut` and for the check before it, and where it
/// starts. A window that does not start at the field's start holds
/// more than the limit, so `cut` drops its partial first word.
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
    let start = selection.start.saturating_sub(CHECK_BEFORE_UNITS).min(end);
    if !in_time() {
        return None;
    }
    let text = focused.string_for_range(Utf16Range {
        start,
        len: end - start,
    })?;
    Some((text, start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    fn at(start: usize) -> Utf16Range {
        Utf16Range { start, len: 0 }
    }

    fn excerpt(before: &str, after: &str) -> Option<FieldExcerpt> {
        Some(FieldExcerpt {
            before: before.to_string(),
            after: after.to_string(),
        })
    }

    #[test]
    fn a_short_field_is_kept_whole_on_both_sides() {
        assert_eq!(
            cut("Hello Priya, see you", at(6)),
            excerpt("Hello ", "Priya, see you")
        );
        assert_eq!(cut("Hello", at(0)), excerpt("", "Hello"));
        assert_eq!(cut("Hello", at(5)), excerpt("Hello", ""));
    }

    #[test]
    fn the_text_before_is_cut_after_the_partial_word() {
        let text = "abcdefg ".repeat(50);
        let cut = cut(&text, at(400)).unwrap();
        assert_eq!(cut.before, "abcdefg ".repeat(37));
        assert!(cut.before.chars().count() <= MAX_BEFORE_CHARS);
        assert_eq!(cut.after, "");
    }

    #[test]
    fn a_whole_word_at_the_limit_is_kept() {
        let text = "alpha ".repeat(100);
        assert_eq!(cut(&text, at(600)).unwrap().before, "alpha ".repeat(50));
    }

    #[test]
    fn the_text_after_is_cut_before_the_partial_word() {
        let text = "abcdefg ".repeat(50);
        let cut = cut(&text, at(0)).unwrap();
        assert_eq!(cut.before, "");
        assert_eq!(cut.after, "abcdefg ".repeat(12).trim_end());
    }

    #[test]
    fn a_long_run_without_whitespace_is_left_out() {
        assert_eq!(cut(&"x".repeat(500), at(500)), None);
        let text = format!("{} hi", "x".repeat(500));
        assert_eq!(cut(&text, at(500)), excerpt("", " hi"));
        let text = format!("hi {}", "y".repeat(500));
        assert_eq!(cut(&text, at(3)), excerpt("hi ", ""));
    }

    #[test]
    fn the_selection_is_left_out() {
        let hello = Utf16Range { start: 6, len: 6 };
        assert_eq!(cut("Hello brave world", hello), excerpt("Hello ", "world"));
    }

    #[test]
    fn offsets_past_the_end_are_clamped() {
        assert_eq!(cut("Hi there", at(50)), excerpt("Hi there", ""));
        let long = Utf16Range { start: 3, len: 100 };
        assert_eq!(cut("Hi there", long), excerpt("Hi ", ""));
    }

    #[test]
    fn utf16_offsets_land_on_char_boundaries() {
        // "Café " is 5 units, the emoji the next 2.
        let text = "Café 😀 日本語 テキスト";
        assert_eq!(cut(text, at(5)), excerpt("Café ", "😀 日本語 テキスト"));
        assert_eq!(cut(text, at(6)), excerpt("Café ", "😀 日本語 テキスト"));
        assert_eq!(cut(text, at(7)), excerpt("Café 😀", " 日本語 テキスト"));
        assert_eq!(cut(text, at(9)), excerpt("Café 😀 日", "本語 テキスト"));

        let emoji = "😀 ".repeat(200);
        assert_eq!(cut(&emoji, at(600)).unwrap().before, "😀 ".repeat(150));
        let accents = "été déjà ".repeat(60);
        let accented = cut(&accents, at(accents.encode_utf16().count())).unwrap();
        assert_eq!(accented.before, "été déjà ".repeat(33));
        assert_eq!(cut(&"日本語".repeat(200), at(600)), None);
        assert_eq!(cut("日本語", at(1)), excerpt("日", "本語"));
    }

    #[test]
    fn a_blank_field_gives_no_excerpt() {
        assert_eq!(cut("", at(0)), None);
        assert_eq!(cut("   \n ", at(2)), None);
        assert_eq!(cut("  hello  ", Utf16Range { start: 2, len: 5 }), None);
    }

    #[test]
    fn debug_shows_lengths_only() {
        let printed = format!("{:?}", excerpt("Dear Siobhán", "regards").unwrap());
        assert!(!printed.contains("Siobhán") && !printed.contains("regards"));
        assert!(printed.contains("12") && printed.contains('7'));
    }

    /// A focused element backed by `text`, recording every text read.
    #[derive(Default)]
    struct Fake {
        secure: bool,
        text: String,
        selection: Option<Utf16Range>,
        range_fails: bool,
        /// How long AXStringForRange takes, as in a slow app.
        delay: Duration,
        reads: RefCell<Vec<&'static str>>,
        requested: RefCell<Option<Utf16Range>>,
    }

    impl Fake {
        fn new(text: &str, selection: Utf16Range) -> Self {
            Self {
                text: text.to_string(),
                selection: Some(selection),
                ..Self::default()
            }
        }
    }

    impl FocusedText for Fake {
        fn is_secure(&self) -> bool {
            self.secure
        }
        fn selection(&self) -> Option<Utf16Range> {
            self.reads.borrow_mut().push("selection");
            self.selection
        }
        fn char_count(&self) -> Option<usize> {
            self.reads.borrow_mut().push("char_count");
            Some(self.text.encode_utf16().count())
        }
        fn string_for_range(&self, range: Utf16Range) -> Option<String> {
            self.reads.borrow_mut().push("string_for_range");
            *self.requested.borrow_mut() = Some(range);
            std::thread::sleep(self.delay);
            if self.range_fails {
                return None;
            }
            let units: Vec<u16> = self.text.encode_utf16().collect();
            units
                .get(range.start..range.start + range.len)
                .map(String::from_utf16_lossy)
        }
        fn value(&self) -> Option<String> {
            self.reads.borrow_mut().push("value");
            Some(self.text.clone())
        }
    }

    fn read(focused: &Fake, deadline: Instant) -> Option<FieldExcerpt> {
        read_field(focused, deadline, |_| false)
    }

    #[test]
    fn a_secure_field_is_never_read() {
        let fake = Fake {
            secure: true,
            ..Fake::new("hunter2 is my password", at(7))
        };
        assert_eq!(read(&fake, later()), None);
        assert!(fake.reads.borrow().is_empty(), "{:?}", fake.reads.borrow());
    }

    #[test]
    fn without_a_selection_nothing_is_guessed() {
        let fake = Fake {
            selection: None,
            ..Fake::new("Hello there", at(0))
        };
        assert_eq!(read(&fake, later()), None);
        assert_eq!(*fake.reads.borrow(), vec!["selection"]);
    }

    #[test]
    fn a_window_around_the_cursor_gives_the_same_excerpt_as_the_whole_field() {
        let texts = [
            "The quick brown fox jumps over the lazy dog, again and again. ".repeat(40),
            "😀 Café crème, 日本語 テキスト ok. ".repeat(60),
            "short field".to_string(),
        ];
        for text in &texts {
            let units = text.encode_utf16().count();
            for start in (0..=units + 3).step_by(37) {
                for len in [0, 5] {
                    let selection = Utf16Range { start, len };
                    let fake = Fake::new(text, selection);
                    assert_eq!(
                        read(&fake, later()),
                        cut(text, selection),
                        "start {start} len {len}"
                    );
                    assert!(!fake.reads.borrow().contains(&"value"));
                }
            }
        }
    }

    #[test]
    fn the_window_asked_for_is_bounded_by_the_limits() {
        let text = "word ".repeat(1_000);
        let selection = Utf16Range {
            start: 3_000,
            len: 10,
        };
        let fake = Fake::new(&text, selection);
        read(&fake, later());
        assert_eq!(
            *fake.requested.borrow(),
            Some(Utf16Range {
                start: 3_000 - CHECK_BEFORE_UNITS,
                len: CHECK_BEFORE_UNITS + 10 + 2 * MAX_AFTER_CHARS + 2,
            })
        );
    }

    #[test]
    fn a_failed_range_read_falls_back_to_the_value() {
        let text = "Thanks for the draft. ";
        let fake = Fake {
            range_fails: true,
            ..Fake::new(text, at(22))
        };
        assert_eq!(read(&fake, later()), excerpt(text, ""));
        assert!(fake.reads.borrow().contains(&"value"));
    }

    #[test]
    fn no_read_starts_once_the_budget_is_spent() {
        let text = "Thanks for the draft. ";
        let slow = Fake {
            range_fails: true,
            delay: Duration::from_millis(30),
            ..Fake::new(text, at(22))
        };
        assert_eq!(
            read(&slow, Instant::now() + Duration::from_millis(10)),
            None
        );
        assert_eq!(
            *slow.reads.borrow(),
            vec!["selection", "char_count", "string_for_range"]
        );

        let late = Fake::new(text, at(22));
        assert_eq!(read(&late, Instant::now()), None);
        assert!(late.reads.borrow().is_empty(), "{:?}", late.reads.borrow());
    }
}
