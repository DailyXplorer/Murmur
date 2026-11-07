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

#[test]
fn a_secure_field_is_never_read() {
    let fake = Fake {
        secure: true,
        ..Fake::new("hunter2 is my password", at(7))
    };
    assert_eq!(read_field(&fake, later()), None);
    assert!(fake.reads.borrow().is_empty(), "{:?}", fake.reads.borrow());
}

fn preceding_chars(fake: &Fake) -> Option<(char, Option<char>)> {
    char_before(fake, later()).map(|before| (before.last, before.previous))
}

#[test]
fn a_mid_word_caret_reads_only_the_characters_before_it() {
    let fake = Fake::new("already written", at(10));
    assert_eq!(preceding_chars(&fake), Some(('r', Some('w'))));
    assert_eq!(
        *fake.requested.borrow(),
        Some(Utf16Range { start: 0, len: 10 })
    );
    assert_eq!(*fake.reads.borrow(), ["selection", "string_for_range"]);
}

#[test]
fn leading_space_never_reads_a_selection_or_guesses_a_caret() {
    for selection in [None, Some(at(0)), Some(Utf16Range { start: 2, len: 3 })] {
        let fake = Fake {
            selection,
            ..Fake::new("Hello", at(2))
        };
        assert!(char_before(&fake, later()).is_none());
        assert_eq!(*fake.reads.borrow(), ["selection"]);
    }
}

#[test]
fn leading_space_never_reads_a_secure_field() {
    let fake = Fake {
        secure: true,
        ..Fake::new("private", at(3))
    };
    assert!(char_before(&fake, later()).is_none());
    assert!(fake.reads.borrow().is_empty());
}

#[test]
fn leading_space_keeps_surrogate_pairs_and_elision_letters_whole() {
    for (text, expected) in [
        ("😀", ('😀', None)),
        ("a😀", ('😀', Some('a'))),
        ("😀😃", ('😃', Some('😀'))),
        ("😀a😃", ('😃', Some('a'))),
        ("qu’", ('’', Some('u'))),
        ("l’", ('’', Some('l'))),
        ("\u{FFFD}😃", ('😃', None)),
    ] {
        let fake = Fake::new(text, at(text.encode_utf16().count()));
        assert_eq!(preceding_chars(&fake), Some(expected), "{text}");
    }
}

#[test]
fn leading_space_reads_grapheme_bases_before_the_caret() {
    for (text, expected) in [
        ("déjà", ('à', Some('j'))),
        ("de\u{301}ja\u{300}", ('a', Some('j'))),
        ("1\u{FE0F}\u{20E3}", ('1', None)),
        (
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
            ('\u{1F3F4}', None),
        ),
        ("👨‍👩‍👧", ('👨', None)),
        ("❤️", ('❤', None)),
        ("👍🏽", ('👍', None)),
    ] {
        let fake = Fake::new(text, at(text.encode_utf16().count()));
        assert_eq!(preceding_chars(&fake), Some(expected), "after {text:?}");
    }
}

#[test]
fn leading_space_separates_decomposed_words_and_complete_emoji() {
    for text in [
        "déjà",
        "de\u{301}ja\u{300}",
        "1\u{FE0F}\u{20E3}",
        "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
        "👨‍👩‍👧",
        "❤️",
        "👍🏽",
    ] {
        let fake = Fake::new(text, at(text.encode_utf16().count()));
        let before = char_before(&fake, later()).unwrap();
        assert!(
            crate::leading_space::needs_leading_space(before, "next"),
            "after {text:?}"
        );
    }
}

#[test]
fn leading_space_separates_keycap_emoji_but_not_plain_symbols() {
    for (text, expected) in [
        ("#️⃣", true),
        ("*️⃣", true),
        ("0️⃣", true),
        ("#", false),
        ("*", false),
        ("#\u{FE0E}", false),
    ] {
        let fake = Fake::new(text, at(text.encode_utf16().count()));
        let before = char_before(&fake, later()).unwrap();
        assert_eq!(
            crate::leading_space::needs_leading_space(before, "next"),
            expected,
            "after {text:?}"
        );
    }
}

#[test]
fn leading_space_keeps_elisions_after_combining_letters() {
    for text in ["l’", "qu’", "l\u{301}’", "l\u{335}’", "qu\u{301}’"] {
        let fake = Fake::new(text, at(text.encode_utf16().count()));
        let before = char_before(&fake, later()).unwrap();
        assert!(
            !crate::leading_space::needs_leading_space(before, "été"),
            "after {text:?}"
        );
    }
}

#[test]
fn leading_space_does_not_guess_a_previous_base_at_a_cut_window_edge() {
    for text in [
        format!("l{}’", "\u{301}".repeat(31)),
        format!("xl{}’", "\u{301}".repeat(30)),
        format!("😀{}’", "\u{301}".repeat(30)),
    ] {
        let fake = Fake::new(&text, at(text.encode_utf16().count()));
        assert_eq!(preceding_chars(&fake), None, "after {text:?}");
    }

    for text in [
        format!("l{}’", "\u{301}".repeat(30)),
        format!("{}l\u{301}’", "x".repeat(40)),
    ] {
        let fake = Fake::new(&text, at(text.encode_utf16().count()));
        assert_eq!(
            preceding_chars(&fake),
            Some(('’', Some('l'))),
            "after {text:?}"
        );
    }
}

#[test]
fn leading_space_does_not_insert_after_a_truncated_elision() {
    let text = format!("l{}’", "\u{301}".repeat(31));
    let fake = Fake::new(&text, at(text.encode_utf16().count()));
    let before = char_before(&fake, later());
    assert!(
        !before.is_some_and(|before| crate::leading_space::needs_leading_space(before, "été")),
        "a cut elision must leave the dictated text unchanged"
    );
}

#[test]
fn leading_space_preserves_decisions_with_complete_caret_context() {
    for (text, previous, expected) in [
        ("’".to_string(), None, true),
        (format!("{}l’", "x".repeat(40)), Some('l'), false),
        (format!("{}word", "x".repeat(40)), Some('r'), true),
    ] {
        let fake = Fake::new(&text, at(text.encode_utf16().count()));
        let before = char_before(&fake, later()).unwrap();
        assert_eq!(before.previous, previous, "after {text:?}");
        assert_eq!(
            crate::leading_space::needs_leading_space(before, "été"),
            expected,
            "after {text:?}"
        );
    }
}

#[test]
fn leading_space_never_uses_a_split_surrogate_as_a_base() {
    assert_eq!(preceding_chars(&Fake::new("😀", at(1))), None);
    let text = format!("😀{}", "a".repeat(31));
    let fake = Fake::new(&text, at(text.encode_utf16().count()));
    assert_eq!(preceding_chars(&fake), Some(('a', Some('a'))));
}

#[test]
fn leading_space_bounds_the_grapheme_window_in_utf16_units() {
    let fake = Fake::new(&"word ".repeat(1_000), at(2_000));
    assert_eq!(preceding_chars(&fake), Some((' ', Some('d'))));
    assert_eq!(
        *fake.requested.borrow(),
        Some(Utf16Range {
            start: 1_968,
            len: 32,
        })
    );
    assert_eq!(*fake.reads.borrow(), ["selection", "string_for_range"]);
}

#[test]
fn leading_space_keeps_two_long_emoji_in_the_bounded_window() {
    let flag = "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}";
    for (suffix, expected) in [
        (format!("{flag}{flag}"), ('\u{1F3F4}', Some('\u{1F3F4}'))),
        (format!("{flag}👨‍👩‍👧‍👦"), ('👨', Some('\u{1F3F4}'))),
    ] {
        let text = format!("{}{suffix}", "x".repeat(40));
        let fake = Fake::new(&text, at(text.encode_utf16().count()));
        assert_eq!(preceding_chars(&fake), Some(expected), "after {suffix:?}");
    }
}

#[test]
fn a_failed_leading_space_range_read_never_reads_the_whole_value() {
    let fake = Fake {
        range_fails: true,
        ..Fake::new("Hello", at(5))
    };
    assert!(char_before(&fake, later()).is_none());
    assert_eq!(*fake.reads.borrow(), ["selection", "string_for_range"]);
}

#[test]
fn leading_space_starts_no_read_after_its_deadline() {
    let fake = Fake::new("Hello", at(5));
    assert!(char_before(&fake, Instant::now()).is_none());
    assert!(fake.reads.borrow().is_empty());
}

#[test]
fn without_a_selection_nothing_is_guessed() {
    let fake = Fake {
        selection: None,
        ..Fake::new("Hello there", at(0))
    };
    assert_eq!(read_field(&fake, later()), None);
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
                    read_field(&fake, later()),
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
        start: 2_000,
        len: 10,
    };
    let fake = Fake::new(&text, selection);
    read_field(&fake, later());
    assert_eq!(
        *fake.requested.borrow(),
        Some(Utf16Range {
            start: 2_000 - (2 * MAX_BEFORE_CHARS + 2),
            len: 2 * MAX_BEFORE_CHARS + 2 + 10 + 2 * MAX_AFTER_CHARS + 2,
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
    assert_eq!(read_field(&fake, later()), excerpt(text, ""));
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
        read_field(&slow, Instant::now() + Duration::from_millis(10)),
        None
    );
    assert_eq!(
        *slow.reads.borrow(),
        vec!["selection", "char_count", "string_for_range"]
    );

    let late = Fake::new(text, at(22));
    assert_eq!(read_field(&late, Instant::now()), None);
    assert!(late.reads.borrow().is_empty(), "{:?}", late.reads.borrow());
}

/// The Fake as UI Automation shows it: text on each side of the
/// selection, without offsets.
impl SelectionText for Fake {
    type Selection = Utf16Range;
    fn is_secure(&self) -> bool {
        self.secure
    }
    fn selection(&self) -> Option<Utf16Range> {
        FocusedText::selection(self)
    }
    fn is_caret(&self, selection: &Utf16Range) -> Option<bool> {
        self.reads.borrow_mut().push("is_caret");
        Some(selection.len == 0)
    }
    fn before(&self, selection: &Utf16Range, max: usize) -> Option<String> {
        self.reads.borrow_mut().push("before");
        std::thread::sleep(self.delay);
        let before = &self.text[..byte_offset(&self.text, selection.start)];
        let skip = before.chars().count().saturating_sub(max);
        Some(before.chars().skip(skip).collect())
    }
    fn after(&self, selection: &Utf16Range, max: usize) -> Option<String> {
        self.reads.borrow_mut().push("after");
        let end = byte_offset(&self.text, selection.start + selection.len);
        Some(self.text[end..].chars().take(max).collect())
    }
}

fn caret_chars(fake: &Fake) -> Option<(char, Option<char>)> {
    char_before_selection(fake, later()).map(|before| (before.last, before.previous))
}

#[test]
fn a_caret_reads_the_graphemes_before_it_from_the_text_next_to_it() {
    let fake = Fake::new("already written", at(10));
    assert_eq!(caret_chars(&fake), Some(('r', Some('w'))));
    assert_eq!(*fake.reads.borrow(), ["selection", "is_caret", "before"]);
    for (text, expected) in [
        ("déjà", Some(('à', Some('j')))),
        ("de\u{301}ja\u{300}", Some(('a', Some('j')))),
        ("1\u{FE0F}\u{20E3}", Some(('1', None))),
        ("👨‍👩‍👧", Some(('👨', None))),
        ("qu’", Some(('’', Some('u')))),
        ("", None),
    ] {
        let fake = Fake::new(text, at(text.encode_utf16().count()));
        assert_eq!(caret_chars(&fake), expected, "after {text:?}");
    }
}

#[test]
fn a_caret_at_the_start_of_the_field_reads_nothing_to_space_after() {
    let fake = Fake::new("Hello", at(0));
    assert_eq!(caret_chars(&fake), None);
}

#[test]
fn a_selection_is_never_read_for_its_text() {
    let fake = Fake::new("Hello world", Utf16Range { start: 2, len: 3 });
    assert_eq!(caret_chars(&fake), None);
    assert_eq!(*fake.reads.borrow(), ["selection", "is_caret"]);

    let no_selection = Fake {
        selection: None,
        ..Fake::new("Hello", at(2))
    };
    assert_eq!(caret_chars(&no_selection), None);
    assert_eq!(*no_selection.reads.borrow(), ["selection"]);
}

#[test]
fn a_secure_field_gives_no_caret_context() {
    let fake = Fake {
        secure: true,
        ..Fake::new("private", at(3))
    };
    assert_eq!(caret_chars(&fake), None);
    assert!(fake.reads.borrow().is_empty());
}

#[test]
fn a_full_window_does_not_guess_a_letter_cut_at_its_left_edge() {
    for text in [
        format!("xl{}’", "\u{301}".repeat(40)),
        format!("😀{}’", "\u{301}".repeat(40)),
    ] {
        let fake = Fake::new(&text, at(text.encode_utf16().count()));
        assert_eq!(caret_chars(&fake), None, "after {text:?}");
    }
    let text = format!("{}l\u{301}’", "x".repeat(40));
    let fake = Fake::new(&text, at(text.encode_utf16().count()));
    assert_eq!(caret_chars(&fake), Some(('’', Some('l'))));
}

#[test]
fn a_short_field_is_not_taken_for_a_cut_window() {
    let fake = Fake::new("l’", at(2));
    assert_eq!(caret_chars(&fake), Some(('’', Some('l'))));
}

#[test]
fn caret_context_starts_no_read_after_its_deadline() {
    let late = Fake::new("Hello", at(5));
    assert!(char_before_selection(&late, Instant::now()).is_none());
    assert!(late.reads.borrow().is_empty());
}

#[test]
fn the_text_on_each_side_gives_the_same_excerpt_as_the_whole_field() {
    let texts = [
        "The quick brown fox jumps over the lazy dog, again and again. ".repeat(40),
        "😀 Café crème, 日本語 テキスト ok. ".repeat(60),
        "short field".to_string(),
    ];
    for text in &texts {
        let units = text.encode_utf16().count();
        for start in (0..=units).step_by(37) {
            for len in [0, 5] {
                let selection = Utf16Range { start, len };
                let fake = Fake::new(text, selection);
                assert_eq!(
                    read_around_selection(&fake, later()),
                    cut(text, selection),
                    "start {start} len {len}"
                );
            }
        }
    }
}

#[test]
fn a_secure_field_or_no_selection_reads_no_text() {
    let secure = Fake {
        secure: true,
        ..Fake::new("hunter2 is my password", at(7))
    };
    assert_eq!(read_around_selection(&secure, later()), None);
    assert!(secure.reads.borrow().is_empty(), "{:?}", secure.reads);

    let no_selection = Fake {
        selection: None,
        ..Fake::new("Hello there", at(0))
    };
    assert_eq!(read_around_selection(&no_selection, later()), None);
    assert_eq!(*no_selection.reads.borrow(), vec!["selection"]);
}

#[test]
fn no_side_is_read_once_the_budget_is_spent() {
    let text = "Thanks for the draft. See you";
    let slow = Fake {
        delay: Duration::from_millis(30),
        ..Fake::new(text, at(22))
    };
    assert_eq!(
        read_around_selection(&slow, Instant::now() + Duration::from_millis(10)),
        None
    );
    assert_eq!(*slow.reads.borrow(), vec!["selection", "before"]);

    let late = Fake::new(text, at(22));
    assert_eq!(read_around_selection(&late, Instant::now()), None);
    assert!(late.reads.borrow().is_empty(), "{:?}", late.reads.borrow());
}
