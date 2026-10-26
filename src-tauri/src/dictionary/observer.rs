//! Watches the field Murmur just pasted into, so a correction the user makes
//! there can teach the dictionary. The field's text stays in this thread's
//! memory: it is never stored or logged, and only the corrected words leave.

use super::learn::{self, Correction};
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
        let Some(fields) = platform_fields() else {
            return;
        };
        let learned = watch(&fields, &inserted, || {
            GENERATION.load(Ordering::Acquire) == generation
        });
        if !learned.is_empty() {
            super::remember(&app, learned);
        }
    });
}

/// The fields of the frontmost app, as the platform's accessibility API
/// shows them.
trait Fields {
    type Field;
    /// The keyboard focus of the frontmost app, unless it is a password
    /// field or the app is Murmur or a password manager.
    fn focused(&self) -> Option<Self::Field>;
    fn same(&self, a: &Self::Field, b: &Self::Field) -> bool;
    /// The field's whole text. None for a password field, or one holding
    /// more than `max` chars (UTF-16 units on Windows).
    fn text(&self, field: &Self::Field, max: usize) -> Option<String>;
}

/// The corrections the user makes to `inserted` in the focused field
/// within the watch window, while `current` holds and the field keeps the
/// focus.
fn watch<F: Fields>(fields: &F, inserted: &str, current: impl Fn() -> bool) -> Vec<Correction> {
    std::thread::sleep(SETTLE_DELAY);
    let Some(field) = fields.focused() else {
        return Vec::new();
    };
    let read = |field: &F::Field| fields.text(field, MAX_FIELD_CHARS);
    let Some(before) = read(&field) else {
        return Vec::new();
    };
    if !before.contains(inserted.trim()) {
        return Vec::new();
    }

    let started = Instant::now();
    let mut still_focused = true;
    let snapshots = std::iter::from_fn(|| {
        if !still_focused || started.elapsed() >= WATCH_WINDOW {
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
        if !current() {
            return None;
        }
        still_focused = fields
            .focused()
            .is_some_and(|focused| fields.same(&focused, &field));
        read(&field)
    });

    settled_edit(&before, inserted, snapshots)
        .map(|edited| learn::review(inserted.trim(), &edited))
        .unwrap_or_default()
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

#[cfg(target_os = "macos")]
struct Accessibility;

#[cfg(target_os = "macos")]
fn platform_fields() -> Option<Accessibility> {
    Some(Accessibility)
}

#[cfg(target_os = "macos")]
impl Fields for Accessibility {
    type Field = crate::accessibility::Element;

    fn focused(&self) -> Option<Self::Field> {
        crate::accessibility::focused_text_element()
    }

    fn same(&self, a: &Self::Field, b: &Self::Field) -> bool {
        a == b
    }

    fn text(&self, field: &Self::Field, max: usize) -> Option<String> {
        if field.is_secure() {
            return None;
        }
        field
            .string("AXValue")
            .filter(|value| value.chars().count() <= max)
    }
}

/// A client of this thread, which the watch runs on.
#[cfg(target_os = "windows")]
fn platform_fields() -> Option<crate::uia::Automation> {
    crate::uia::Automation::new()
        .map_err(|e| log::debug!("UI Automation is unavailable for learning: {e}"))
        .ok()
}

#[cfg(target_os = "windows")]
impl Fields for crate::uia::Automation {
    type Field = crate::uia::Element;

    fn focused(&self) -> Option<Self::Field> {
        let app = crate::uia::frontmost_app()?;
        self.focus_in(app.pid)
            .filter(|focused| !focused.is_secure())
    }

    fn same(&self, a: &Self::Field, b: &Self::Field) -> bool {
        crate::uia::Automation::same(self, a, b)
    }

    /// Line breaks as the dictation wrote them: Windows fields hold `\r\n`
    /// or `\r` for each `\n` pasted.
    fn text(&self, field: &Self::Field, max: usize) -> Option<String> {
        field
            .text(max)
            .map(|text| text.replace("\r\n", "\n").replace('\r', "\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

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

    #[test]
    fn a_field_emptied_before_any_edit_teaches_nothing() {
        let inserted = "I talked to clawd about it";
        assert!(learned(inserted, inserted, &["", "I talked to Claude about it"]).is_empty());
    }

    const INSERTED: &str = "I talked to clawd about it";
    const CORRECTED: &str = "I talked to Claude about it";

    fn claude() -> Vec<Correction> {
        vec![Correction {
            word: "Claude".to_string(),
            heard_as: "clawd".to_string(),
        }]
    }

    /// Field 1's text and which field holds the focus, one step for each
    /// time the watch looks at the focus.
    struct Timeline {
        steps: Vec<(Option<u8>, &'static str)>,
        step: Cell<usize>,
    }

    impl Timeline {
        fn new(steps: &[(Option<u8>, &'static str)]) -> Self {
            Self {
                steps: steps.to_vec(),
                step: Cell::new(0),
            }
        }
    }

    impl Fields for Timeline {
        type Field = u8;

        fn focused(&self) -> Option<u8> {
            let step = self.step.get();
            self.step.set(step + 1);
            self.steps.get(step)?.0
        }

        fn same(&self, a: &u8, b: &u8) -> bool {
            a == b
        }

        fn text(&self, field: &u8, _: usize) -> Option<String> {
            let (_, text) = self.steps.get(self.step.get().checked_sub(1)?)?;
            (*field == 1).then(|| text.to_string())
        }
    }

    #[test]
    fn the_watch_learns_a_correction_in_the_focused_field() {
        let fields = Timeline::new(&[
            (Some(1), INSERTED),
            (Some(1), CORRECTED),
            (Some(1), CORRECTED),
            (Some(1), ""),
        ]);
        assert_eq!(watch(&fields, INSERTED, || true), claude());
    }

    #[test]
    fn the_watch_ends_when_the_focus_leaves_the_field() {
        let fields = Timeline::new(&[
            (Some(1), INSERTED),
            (Some(2), INSERTED),
            (Some(1), CORRECTED),
            (Some(1), CORRECTED),
            (Some(1), ""),
        ]);
        assert!(watch(&fields, INSERTED, || true).is_empty());
    }

    #[test]
    fn the_next_dictation_ends_the_watch() {
        let fields = Timeline::new(&[
            (Some(1), INSERTED),
            (Some(1), INSERTED),
            (Some(1), CORRECTED),
            (Some(1), CORRECTED),
            (Some(1), ""),
        ]);
        let polls = Cell::new(0);
        let current = || {
            polls.set(polls.get() + 1);
            polls.get() < 2
        };
        assert!(watch(&fields, INSERTED, current).is_empty());
    }

    #[test]
    fn a_field_without_the_dictation_is_not_watched() {
        let fields = Timeline::new(&[
            (Some(1), "Something else"),
            (Some(1), CORRECTED),
            (Some(1), CORRECTED),
        ]);
        assert!(watch(&fields, INSERTED, || true).is_empty());
        assert_eq!(fields.step.get(), 1, "one look, then the watch ended");
    }

    /// The watch on real Win32 controls, in windows this test opens, so no
    /// foreground window is needed. The test plays the user: it changes
    /// the field between the watch's reads.
    #[cfg(target_os = "windows")]
    mod uia {
        use super::*;
        use crate::uia::test_fields::{self, requests, Read, PLAIN_TEXT};
        use crate::uia::test_window::{child, TestWindow};
        use crate::uia::{Automation, Element};
        use std::sync::mpsc::{self, Sender};
        use windows::core::HSTRING;
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::System::LibraryLoader::LoadLibraryW;
        use windows::Win32::UI::WindowsAndMessaging::{
            SendMessageW, ES_AUTOHSCROLL, ES_MULTILINE, WINDOW_STYLE, WM_SETTEXT, WS_BORDER,
        };

        /// The rich edit control of Msftedit.dll, as in Windows 11's Notepad.
        const RICH_EDIT: &str = "RICHEDIT50W";

        fn field_window(class: &'static str, style: u32, text: &'static str) -> TestWindow {
            if class == RICH_EDIT {
                // SAFETY: loads a system library that registers the class.
                unsafe { LoadLibraryW(windows::core::w!("Msftedit.dll")) }.expect("Msftedit.dll");
            }
            TestWindow::open(move |window| {
                let style = WINDOW_STYLE(style) | WS_BORDER;
                vec![child(window, class, text, style).0 as isize]
            })
        }

        fn set_text(field: isize, text: &str) {
            let text = HSTRING::from(text);
            // SAFETY: the field is a live window, whose thread pumps
            // messages; the text outlives the call.
            unsafe {
                SendMessageW(
                    HWND(field as *mut _),
                    WM_SETTEXT,
                    Some(WPARAM(0)),
                    Some(LPARAM(text.as_ptr() as isize)),
                )
            };
        }

        /// The test field as the focus, read and compared by the production
        /// client. Each read is told to the test.
        struct TestField {
            automation: Automation,
            field: isize,
            reads: Sender<Option<String>>,
        }

        impl Fields for TestField {
            type Field = Element;

            fn focused(&self) -> Option<Element> {
                self.automation.window(self.field)
            }

            fn same(&self, a: &Element, b: &Element) -> bool {
                Fields::same(&self.automation, a, b)
            }

            fn text(&self, field: &Element, max: usize) -> Option<String> {
                let text = Fields::text(&self.automation, field, max);
                let _ = self.reads.send(text.clone());
                text
            }
        }

        /// Watches the field in `window` and changes it to each of `edits`
        /// in turn, once the watch has read the field before them and read
        /// each earlier edit twice, which settles it. Also returns every
        /// read.
        fn learned(window: &TestWindow, edits: &[&str]) -> (Vec<Correction>, Vec<Option<String>>) {
            let field = window.handles[0];
            let (reads, read) = mpsc::channel();
            let watching = std::thread::spawn(move || {
                let fields = TestField {
                    automation: Automation::new().expect("UI Automation client"),
                    field,
                    reads,
                };
                watch(&fields, INSERTED, || true)
            });
            let mut seen = vec![read.recv().expect("the field before any edit")];
            for (index, edit) in edits.iter().enumerate() {
                set_text(field, edit);
                if index + 1 == edits.len() {
                    break;
                }
                while seen
                    .iter()
                    .rev()
                    .take(2)
                    .filter(|text| text.as_deref().map(str::trim_end) == Some(*edit))
                    .count()
                    < 2
                {
                    seen.push(
                        read.recv()
                            .expect("the watch ended before the edit settled"),
                    );
                }
            }
            let corrections = watching.join().unwrap();
            seen.extend(read.try_iter());
            (corrections, seen)
        }

        #[test]
        fn a_word_corrected_in_a_field_is_learned_once() {
            for (class, style) in [
                ("EDIT", ES_AUTOHSCROLL as u32),
                ("EDIT", (ES_MULTILINE | ES_AUTOHSCROLL) as u32),
                (RICH_EDIT, (ES_MULTILINE | ES_AUTOHSCROLL) as u32),
            ] {
                let window = field_window(class, style, INSERTED);
                let (corrections, _) = learned(&window, &[CORRECTED, ""]);
                assert_eq!(corrections, claude(), "{class} {style:#x}");
            }
        }

        /// A rich edit's document ends with a paragraph mark, which reads
        /// as a last line break.
        #[test]
        fn a_field_keeps_the_line_breaks_of_the_dictation() {
            let automation = Automation::new().expect("UI Automation client");
            for (class, expected) in [
                ("EDIT", "Dear Ingrid,\nthanks"),
                (RICH_EDIT, "Dear Ingrid,\nthanks\n"),
            ] {
                let window = field_window(
                    class,
                    (ES_MULTILINE | ES_AUTOHSCROLL) as u32,
                    "Dear Ingrid,\r\nthanks",
                );
                let field = automation.window(window.handles[0]).unwrap();
                assert_eq!(
                    Fields::text(&automation, &field, MAX_FIELD_CHARS).as_deref(),
                    Some(expected),
                    "{class}"
                );
            }
        }

        /// Both controls give a Value. The rich edit also has the text
        /// pattern a document gives instead, which is read only up to the
        /// limit.
        #[test]
        fn a_field_longer_than_the_limit_is_not_read() {
            let automation = Automation::new().expect("UI Automation client");
            let window = field_window(RICH_EDIT, (ES_MULTILINE | ES_AUTOHSCROLL) as u32, INSERTED);
            let field = automation.window(window.handles[0]).unwrap();
            let document = field.document_text(MAX_FIELD_CHARS).unwrap();
            assert_eq!(document.trim_end(), INSERTED);
            let len = document.encode_utf16().count();
            assert_eq!(field.document_text(len), Some(document));
            assert_eq!(field.document_text(len - 1), None);
            drop(window);

            for class in ["EDIT", RICH_EDIT] {
                let window = field_window(class, (ES_MULTILINE | ES_AUTOHSCROLL) as u32, INSERTED);
                let field = automation.window(window.handles[0]).unwrap();
                let whole = Fields::text(&automation, &field, MAX_FIELD_CHARS).unwrap();
                assert_eq!(whole.trim_end(), INSERTED, "{class}");
                let len = whole.encode_utf16().count();
                assert_eq!(
                    Fields::text(&automation, &field, len).as_deref(),
                    Some(whole.as_str()),
                    "{class}"
                );
                assert_eq!(Fields::text(&automation, &field, len - 1), None, "{class}");
            }
        }

        /// The password field would give its text by Value and by text
        /// pattern, and counts each request; the plain field beside it
        /// shows that both are counted.
        #[test]
        fn a_password_fields_text_is_never_requested() {
            let window = test_fields::open();
            let (corrections, reads) = learned(&window, &[CORRECTED]);
            let automation = Automation::new().expect("UI Automation client");
            let secret = automation.window(window.handles[0]).unwrap();
            let document = secret.document_text(MAX_FIELD_CHARS);
            assert_eq!(
                [requests(true, Read::Value), requests(true, Read::Text)],
                [0, 0],
                "the password field's text"
            );
            assert!(secret.is_secure());
            assert!(corrections.is_empty());
            assert_eq!(reads, [None], "nothing read, and the watch ended");
            assert_eq!(document, None);

            let plain = automation.window(window.handles[1]).unwrap();
            assert_eq!(
                Fields::text(&automation, &plain, MAX_FIELD_CHARS).as_deref(),
                Some(PLAIN_TEXT)
            );
            assert!(requests(false, Read::Value) > 0);
            assert_eq!(
                plain.document_text(MAX_FIELD_CHARS).as_deref(),
                Some(PLAIN_TEXT)
            );
            assert!(requests(false, Read::Text) > 0);
        }
    }
}
