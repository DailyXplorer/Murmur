//! What the window the user dictates into offers the cleanup step: proper
//! names visible in it, as spelling hints, and a short excerpt of the
//! focused field around the insertion point. Both are read through
//! Accessibility on macOS, or UI Automation on Windows, on a background
//! thread when dictation starts, reduced locally, and dropped after use:
//! only the names and the excerpt leave this module, and nothing is stored
//! or logged.

use crate::field_excerpt::FieldExcerpt;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
use {
    crate::accessibility::Element,
    crate::field_excerpt::{read_field, FocusedText, Utf16Range},
};

const MAX_NAMES: usize = 50;
const MAX_NAME_CHARS: usize = 40;
/// Unbroken tokens this long are keys or hashes, not names.
const LONG_TOKEN_CHARS: usize = 20;
const MAX_NODES: usize = 1_500;
const MAX_TEXT_CHARS: usize = 40_000;
const READ_BUDGET: Duration = Duration::from_millis(400);
/// The field is read first, and a slow one must leave the window time.
const FIELD_BUDGET: Duration = Duration::from_millis(200);
/// Stopping must not wait on a slow app; late names are simply not used.
const COLLECT_WAIT: Duration = Duration::from_millis(50);

/// What one dictation learned from the window it was started in.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WindowContext {
    pub names: Vec<String>,
    pub field: Option<FieldExcerpt>,
}

/// Which parts of the frontmost window a dictation reads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextReads {
    /// Names visible anywhere in the focused window.
    pub names: bool,
    /// The excerpt of the focused field around the insertion point.
    pub field: bool,
}

static PENDING: Mutex<Option<Receiver<WindowContext>>> = Mutex::new(None);

/// Starts a dictation: reads what `reads` asks for from the frontmost
/// window, and always drops the context of an earlier dictation nobody
/// claimed, such as one cancelled with Escape.
pub fn capture(reads: ContextReads) {
    capture_with(reads, read_frontmost_window);
}

fn capture_with(
    reads: ContextReads,
    read: impl FnOnce(ContextReads) -> WindowContext + Send + 'static,
) {
    let (sender, receiver) = mpsc::channel();
    let reads_any = reads.names || reads.field;
    if let Ok(mut pending) = PENDING.lock() {
        *pending = reads_any.then_some(receiver);
    }
    if !reads_any {
        return;
    }
    std::thread::spawn(move || {
        let _ = sender.send(read(reads));
    });
}

/// The context captured for the dictation that is ending, if ready.
pub fn collect() -> WindowContext {
    let receiver = PENDING.lock().ok().and_then(|mut pending| pending.take());
    receiver
        .and_then(|receiver| receiver.recv_timeout(COLLECT_WAIT).ok())
        .unwrap_or_default()
}

/// Logs lengths and counts only, never what was read.
#[cfg(target_os = "windows")]
fn read_frontmost_window(reads: ContextReads) -> WindowContext {
    let started = Instant::now();
    let Some(app) = crate::uia::frontmost_app() else {
        return WindowContext::default();
    };
    let automation = match crate::uia::Automation::new() {
        Ok(automation) => automation,
        Err(e) => {
            log::debug!("UI Automation is unavailable for the window context: {e}");
            return WindowContext::default();
        }
    };
    let context = read_app(reads, &automation, app, started);
    log::debug!(
        "Window context: {} names, excerpt {:?}, in {:?}",
        context.names.len(),
        context.field,
        started.elapsed()
    );
    context
}

/// The focused field first, then the window, with what is left of the
/// budget that began at `started`.
#[cfg(target_os = "windows")]
fn read_app(
    reads: ContextReads,
    automation: &crate::uia::Automation,
    app: crate::foreground::ForegroundWindow,
    started: Instant,
) -> WindowContext {
    read_window(
        reads,
        || {
            automation.focus_in(app.pid).and_then(|focused| {
                crate::field_excerpt::read_around_selection(&focused, started + FIELD_BUDGET)
            })
        },
        || {
            automation
                .window(app.window)
                .map(|element| {
                    window_text(
                        UiaNode {
                            automation,
                            element,
                        },
                        started,
                    )
                })
                .unwrap_or_default()
        },
    )
}

/// An element of the window's UI Automation tree, with the client that
/// finds its children.
#[cfg(target_os = "windows")]
struct UiaNode<'a> {
    automation: &'a crate::uia::Automation,
    element: crate::uia::Element,
}

#[cfg(target_os = "windows")]
impl WindowNode for UiaNode<'_> {
    fn is_secure(&self) -> bool {
        self.element.is_secure()
    }

    fn texts(&self) -> Vec<String> {
        [self.element.value(), self.element.name()]
            .into_iter()
            .flatten()
            .collect()
    }

    fn children(&self) -> Vec<Self> {
        self.automation
            .children(&self.element)
            .into_iter()
            .map(|element| UiaNode {
                automation: self.automation,
                element,
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn read_frontmost_window(reads: ContextReads) -> WindowContext {
    let started = Instant::now();
    crate::accessibility::frontmost_app()
        .map(|(pid, app)| read_app(reads, pid, &app, started))
        .unwrap_or_default()
}

/// The focused field first, then the focused window with what is left of
/// the budget that began at `started`.
#[cfg(target_os = "macos")]
fn read_app(reads: ContextReads, pid: i32, app: &Element, started: Instant) -> WindowContext {
    read_window(
        reads,
        || {
            crate::accessibility::focus_in(pid, app)
                .and_then(|focused| read_field(&focused, started + FIELD_BUDGET))
        },
        || {
            app.element("AXFocusedWindow")
                .map(|window| window_text(window, started))
                .unwrap_or_default()
        },
    )
}

/// Runs only the reads `reads` asks for.
fn read_window(
    reads: ContextReads,
    field: impl FnOnce() -> Option<FieldExcerpt>,
    window_text: impl FnOnce() -> String,
) -> WindowContext {
    let field = if reads.field { field() } else { None };
    let names = if reads.names {
        names_in(field.as_ref(), &window_text())
    } else {
        Vec::new()
    };
    WindowContext { names, field }
}

/// Names in the excerpt come first: they are next to what the user is
/// writing.
fn names_in(field: Option<&FieldExcerpt>, window_text: &str) -> Vec<String> {
    let excerpt = field
        .map(|field| format!("{}\n{}", field.before, field.after))
        .unwrap_or_default();
    let mut names = candidate_names(&excerpt);
    for name in candidate_names(window_text) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.truncate(MAX_NAMES);
    names
}

#[cfg(target_os = "macos")]
impl FocusedText for Element {
    fn is_secure(&self) -> bool {
        Element::is_secure(self)
    }

    fn selection(&self) -> Option<Utf16Range> {
        self.range("AXSelectedTextRange")
    }

    fn char_count(&self) -> Option<usize> {
        usize::try_from(self.number("AXNumberOfCharacters")?).ok()
    }

    fn string_for_range(&self, range: Utf16Range) -> Option<String> {
        Element::string_for_range(self, range)
    }

    fn value(&self) -> Option<String> {
        self.string("AXValue")
    }
}

/// One element of a window's tree, as `window_text` walks it.
trait WindowNode: Sized {
    fn is_secure(&self) -> bool;
    fn texts(&self) -> Vec<String>;
    fn children(&self) -> Vec<Self>;
}

#[cfg(target_os = "macos")]
impl WindowNode for Element {
    fn is_secure(&self) -> bool {
        Element::is_secure(self)
    }

    fn texts(&self) -> Vec<String> {
        ["AXValue", "AXTitle", "AXDescription"]
            .iter()
            .filter_map(|attribute| self.string(attribute))
            .collect()
    }

    fn children(&self) -> Vec<Self> {
        Element::children(self)
    }
}

/// The text of `window` and its descendants, breadth first, until the read
/// budget that started at `started` runs out. A secure field is skipped
/// with everything inside it.
fn window_text(window: impl WindowNode, started: Instant) -> String {
    let mut text = String::new();
    let mut queue = VecDeque::from([window]);
    let mut visited = 0;
    while let Some(element) = queue.pop_front() {
        visited += 1;
        if visited > MAX_NODES || text.len() > MAX_TEXT_CHARS || started.elapsed() > READ_BUDGET {
            break;
        }
        if element.is_secure() {
            continue;
        }
        for value in element.texts() {
            if !value.trim().is_empty() {
                text.push_str(&value);
                text.push('\n');
            }
        }
        queue.extend(element.children());
    }
    text
}

/// Words that look like names: capitalised but not merely sentence case,
/// never seen in lowercase in the same text, plus the name parts of email
/// addresses and @handles. Most frequent first, at most 50.
pub fn candidate_names(text: &str) -> Vec<String> {
    let lowercase_words: std::collections::HashSet<String> = text
        .split_whitespace()
        .map(trim_word)
        .filter(|word| {
            word.chars().any(char::is_alphabetic) && !word.chars().any(char::is_uppercase)
        })
        .map(str::to_string)
        .collect();

    let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
    let mut order = 0;
    let mut add = |name: String, counts: &mut HashMap<String, (usize, usize)>| {
        order += 1;
        counts.entry(name).or_insert((0, order)).0 += 1;
    };

    for line in text.lines() {
        let mut sentence_start = true;
        for token in line.split_whitespace() {
            if let Some(parts) = address_names(token) {
                for part in parts {
                    add(part, &mut counts);
                }
                sentence_start = false;
                continue;
            }
            let word = trim_word(token);
            if is_name(word, sentence_start) && !lowercase_words.contains(&word.to_lowercase()) {
                add(word.to_string(), &mut counts);
            }
            sentence_start = token.ends_with(['.', '!', '?', ':', '•', '|']);
        }
    }

    let mut names: Vec<(String, (usize, usize))> = counts.into_iter().collect();
    names.sort_by(|(_, (count_a, first_a)), (_, (count_b, first_b))| {
        count_b.cmp(count_a).then(first_a.cmp(first_b))
    });
    names
        .into_iter()
        .map(|(name, _)| name)
        .take(MAX_NAMES)
        .collect()
}

fn trim_word(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric())
}

fn is_name(word: &str, sentence_start: bool) -> bool {
    let chars = word.chars().count();
    if !(2..=MAX_NAME_CHARS).contains(&chars)
        || !word
            .chars()
            .all(|c| c.is_alphanumeric() || "-'&.".contains(c))
        || word.chars().all(|c| c.is_ascii_digit())
        || word.contains("www.")
        || looks_generated(word)
    {
        return false;
    }
    let Some(first) = word.chars().next() else {
        return false;
    };
    let has_lowercase = word.chars().any(char::is_lowercase);
    let inner_capital = word.chars().skip(1).any(char::is_uppercase);
    if !first.is_uppercase() || !has_lowercase {
        // CamelCase brands that start lowercase ("iPhone", "macOS").
        return inner_capital && has_lowercase;
    }
    inner_capital || !sentence_start
}

/// Generated strings such as keys and hashes rather than words, which would
/// crowd real names out of the hints. Written names mix letters and digits
/// at most once ("GPT-4o", "iPhone15"), switch case at most twice
/// ("McDonald", "DeepSeek"), have vowels, and stay short unless they are
/// hyphenated words.
fn looks_generated(word: &str) -> bool {
    let segments: Vec<&str> = word
        .split(['-', '\'', '&', '.'])
        .filter(|segment| !segment.is_empty())
        .collect();

    let hyphenated_words = segments.iter().all(|segment| {
        segment.chars().all(char::is_alphabetic) && segment.chars().count() < LONG_TOKEN_CHARS
    });
    if word.chars().count() >= LONG_TOKEN_CHARS && !hyphenated_words {
        return true;
    }

    let mut digit_runs = 0;
    let mut letters_after_digits = 0;
    let mut previous_digit = false;
    for c in word.chars().filter(|c| c.is_alphanumeric()) {
        let digit = c.is_numeric();
        if digit && !previous_digit {
            digit_runs += 1;
            letters_after_digits = 0;
        } else if !digit && digit_runs > 0 {
            letters_after_digits += 1;
        }
        previous_digit = digit;
    }
    if digit_runs > 1 || letters_after_digits > 1 {
        return true;
    }

    // Each segment's first letter may be a capital without counting.
    let case_switches: usize = segments
        .iter()
        .map(|segment| {
            let cases: Vec<bool> = segment
                .chars()
                .filter(|c| c.is_uppercase() || c.is_lowercase())
                .map(char::is_uppercase)
                .skip(1)
                .collect();
            cases.windows(2).filter(|pair| pair[0] != pair[1]).count()
        })
        .sum();
    if case_switches > 2 {
        return true;
    }

    let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() >= 8 && letters.iter().all(char::is_ascii_alphabetic) {
        let vowels = letters
            .iter()
            .filter(|c| "aeiouyAEIOUY".contains(**c))
            .count();
        if vowels * 7 < letters.len() {
            return true;
        }
    }

    word.len() >= 8
        && word.chars().all(|c| c.is_ascii_hexdigit())
        && word.chars().any(|c| c.is_ascii_digit())
}

/// "jane.doe@example.com" -> ["Jane", "Doe"]; "@janedoe" -> ["janedoe"].
fn address_names(token: &str) -> Option<Vec<String>> {
    let token = token.trim_matches(|c: char| c.is_ascii_punctuation() && c != '@' && c != '_');
    if let Some(handle) = token.strip_prefix('@') {
        let handle = handle.trim_end_matches(|c: char| !c.is_alphanumeric());
        let valid =
            handle.chars().count() >= 2 && handle.chars().all(|c| c.is_alphanumeric() || c == '_');
        return valid.then(|| vec![handle.to_string()]);
    }
    let (local, domain) = token.split_once('@')?;
    if !domain.contains('.') {
        return None;
    }
    let parts: Vec<String> = local
        .split(['.', '_', '-', '+'])
        .filter(|part| part.chars().count() >= 2 && part.chars().all(char::is_alphabetic))
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| {
                    first
                        .to_uppercase()
                        .chain(chars.flat_map(char::to_lowercase))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    Some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_inside_sentences_are_found_and_sentence_starts_are_not() {
        let text = "Thanks for the update. I spoke with Ingrid and Tobias yesterday.\nShipping is on track.";
        assert_eq!(candidate_names(text), vec!["Ingrid", "Tobias"]);
    }

    #[test]
    fn interface_labels_at_line_start_are_skipped() {
        assert!(candidate_names("File\nEdit\nView\nSearch messages").is_empty());
    }

    #[test]
    fn capitalised_common_words_seen_in_lowercase_are_skipped() {
        let text = "Open the Settings page. the settings are saved with Murmur.";
        assert_eq!(candidate_names(text), vec!["Murmur"]);
    }

    #[test]
    fn camel_case_brands_count_even_at_sentence_start() {
        assert_eq!(
            candidate_names("GitHub and iPhone users. macOS ships soon."),
            vec!["GitHub", "iPhone", "macOS"]
        );
    }

    #[test]
    fn emails_and_handles_give_their_name_parts() {
        assert_eq!(
            candidate_names("reply to jane.doe@example.com or ping @kai_dev today"),
            vec!["Jane", "Doe", "kai_dev"]
        );
    }

    #[test]
    fn acronyms_numbers_and_urls_are_not_names() {
        assert!(candidate_names("see the PDF and 2024 and www.Example.com now").is_empty());
    }

    #[test]
    fn names_are_deduplicated_ranked_by_frequency_and_capped() {
        let text = "meet Zed then Amaya then Zed again";
        assert_eq!(candidate_names(text), vec!["Zed", "Amaya"]);

        let many: String = (0..80).map(|i| format!("with Name{i} ")).collect();
        assert_eq!(candidate_names(&many).len(), MAX_NAMES);
    }

    #[test]
    fn generated_tokens_are_not_names() {
        for token in [
            // Generated passwords.
            "Xk9mPq2vLr7T",
            "Tr0ub4dor&3",
            "hGqWzKpLxV",
            "Xkqzvbnm",
            // Base62, base64 and hex tokens.
            "aB3dE5fG7hJ9kL1mN0pQ",
            "Z7kQ2mN9xR4bL1vT8cW3",
            "dGhpcyBpcyBhIHNlY3JldA==",
            "Deadbeef0042",
            "Fa3c9e1b7d5a",
            // AWS-like access key id and secret key.
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        ] {
            assert!(
                candidate_names(&format!("see {token} here")).is_empty(),
                "accepted {token}"
            );
        }
    }

    #[test]
    fn real_names_and_brands_are_still_names() {
        let text = "met McDonald, O'Brien, Jean-Luc, Schwarzschild and Strzelczyk about iPhone15, \
             GPT-4o, OpenAI and DeepSeek";
        assert_eq!(
            candidate_names(text),
            vec![
                "McDonald",
                "O'Brien",
                "Jean-Luc",
                "Schwarzschild",
                "Strzelczyk",
                "iPhone15",
                "GPT-4o",
                "OpenAI",
                "DeepSeek"
            ]
        );
        for word in [
            "GPT-4",
            "M3",
            "iPhone",
            "Pixel8a",
            "Hewlett-Packard-Enterprise",
        ] {
            assert!(!looks_generated(word), "rejected {word}");
        }
    }

    /// `PENDING` is shared by every dictation, so its tests take turns.
    static PENDING_TESTS: Mutex<()> = Mutex::new(());

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn field(before: &str, after: &str) -> Option<FieldExcerpt> {
        Some(FieldExcerpt {
            before: before.to_string(),
            after: after.to_string(),
        })
    }

    #[test]
    fn a_dictation_started_with_names_off_drops_an_unclaimed_capture() {
        let _turn = PENDING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let (sender, receiver) = mpsc::channel();
        sender
            .send(WindowContext {
                names: names(&["Ingrid"]),
                field: field("Hi Ingrid, ", ""),
            })
            .unwrap();
        *PENDING.lock().unwrap() = Some(receiver);
        capture(ContextReads::default());
        assert_eq!(collect(), WindowContext::default());
    }

    #[test]
    fn each_read_runs_only_when_asked_for() {
        use std::sync::Arc;

        let _turn = PENDING_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let excerpt = || field("Hi Priya, ", "");
        for (reads, expected_reads, expected) in [
            (ContextReads::default(), vec![], WindowContext::default()),
            (
                ContextReads {
                    names: true,
                    field: false,
                },
                vec!["window"],
                WindowContext {
                    names: names(&["Ingrid"]),
                    field: None,
                },
            ),
            (
                ContextReads {
                    names: false,
                    field: true,
                },
                vec!["field"],
                WindowContext {
                    names: Vec::new(),
                    field: excerpt(),
                },
            ),
            (
                ContextReads {
                    names: true,
                    field: true,
                },
                vec!["field", "window"],
                WindowContext {
                    names: names(&["Priya", "Ingrid"]),
                    field: excerpt(),
                },
            ),
        ] {
            let asked = Arc::new(Mutex::new(Vec::new()));
            let log = Arc::clone(&asked);
            capture_with(reads, move |reads| {
                let field_log = Arc::clone(&log);
                let context = read_window(
                    reads,
                    move || {
                        field_log.lock().unwrap().push("field");
                        excerpt()
                    },
                    || {
                        log.lock().unwrap().push("window");
                        "meet Ingrid today".to_string()
                    },
                );
                log.lock().unwrap().push("done");
                context
            });
            let started = Instant::now();
            while !asked.lock().unwrap().contains(&"done")
                && started.elapsed() < Duration::from_millis(200)
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(collect(), expected, "{reads:?}");
            let asked: Vec<_> = asked
                .lock()
                .unwrap()
                .iter()
                .copied()
                .filter(|read| *read != "done")
                .collect();
            assert_eq!(asked, expected_reads, "{reads:?}");
        }
    }

    #[test]
    fn the_excerpt_and_names_are_sent_as_read() {
        let all = ContextReads {
            names: true,
            field: true,
        };
        let context = read_window(
            all,
            || field("Code porte : 4512B\n", ""),
            || "Wi-Fi password: Bluebird\nshared with Corvina".to_string(),
        );
        assert_eq!(
            context,
            WindowContext {
                names: names(&["Wi-Fi", "Corvina"]),
                field: field("Code porte : 4512B\n", ""),
            }
        );
    }

    /// Every text the name finder was checked on, and the names it gives.
    /// Add rows; never change one to make a fix pass.
    const NAMES_CORPUS: &[(&str, &[&str])] = &[
        (
            "Thanks for the update. I spoke with Ingrid and Tobias yesterday.\nShipping is on track.",
            &["Ingrid", "Tobias"],
        ),
        ("File\nEdit\nView\nSearch messages", &[]),
        (
            "Open the Settings page. the settings are saved with Murmur.",
            &["Murmur"],
        ),
        (
            "GitHub and iPhone users. macOS ships soon.",
            &["GitHub", "iPhone", "macOS"],
        ),
        (
            "reply to jane.doe@example.com or ping @kai_dev today",
            &["Jane", "Doe", "kai_dev"],
        ),
        ("see the PDF and 2024 and www.Example.com now", &[]),
        ("meet Zed then Amaya then Zed again", &["Zed", "Amaya"]),
        ("see Xk9mPq2vLr7T here", &[]),
        ("see Tr0ub4dor&3 here", &[]),
        ("see hGqWzKpLxV here", &[]),
        ("see Xkqzvbnm here", &[]),
        ("see aB3dE5fG7hJ9kL1mN0pQ here", &[]),
        ("see Z7kQ2mN9xR4bL1vT8cW3 here", &[]),
        ("see dGhpcyBpcyBhIHNlY3JldA== here", &[]),
        ("see Deadbeef0042 here", &[]),
        ("see Fa3c9e1b7d5a here", &[]),
        ("see AKIAIOSFODNN7EXAMPLE here", &[]),
        ("see wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY here", &[]),
        (
            "met McDonald, O'Brien, Jean-Luc, Schwarzschild and Strzelczyk about iPhone15, \
             GPT-4o, OpenAI and DeepSeek",
            &[
                "McDonald",
                "O'Brien",
                "Jean-Luc",
                "Schwarzschild",
                "Strzelczyk",
                "iPhone15",
                "GPT-4o",
                "OpenAI",
                "DeepSeek",
            ],
        ),
    ];

    #[test]
    fn names_corpus() {
        let wrong: Vec<String> = NAMES_CORPUS
            .iter()
            .filter(|(text, expected)| candidate_names(text) != names(expected))
            .map(|(text, expected)| {
                format!("{text:?}: {:?}, not {expected:?}", candidate_names(text))
            })
            .collect();
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Measures every read on each regular app running, without bringing
    /// any to the front, and writes a markdown table. Only lengths, counts,
    /// roles and attribute names are recorded, never text.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn coverage_of_running_apps() {
        use crate::accessibility::{is_password_app, is_trusted};
        use crate::field_excerpt::{MAX_AFTER_CHARS, MAX_BEFORE_CHARS};
        use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
        use std::fmt::Write as _;

        let out = std::env::var("MURMUR_COVERAGE_OUT")
            .unwrap_or_else(|_| "/tmp/murmur-i153-coverage.md".to_string());
        let chars = |text: Option<String>| {
            text.map_or("-".to_string(), |text| text.chars().count().to_string())
        };
        let mut main = format!(
            "# Window context coverage\n\n{} UTC, Accessibility trusted: {}\n\n\
             Before: the window read at 71afa240 (window chars, names, ms). After: the \
             production reader with names and field on (excerpt chars before/after the \
             cursor, names, ms).\n\n\
             | App | Before | After | Notes |\n|---|---|---|---|\n",
            chrono::Utc::now().format("%Y-%m-%d %H:%M"),
            is_trusted()
        );
        let mut report = String::from(
            "\n## Probe details\n\n\
             (a) selection + AXStringForRange of the reader's window, (b) selection + AXValue, \
             (c) AXSelectedText, (d) AXVisibleCharacterRange length, (e) BFS text of the nearest \
             AXWebArea above the focus. Lengths in chars; `-` is a failed read.\n\n\
             | app | focus role/subrole | secure | selection | (a) | (b) | (c) | (d) | (e) chars, ms \
             | before: window chars, names, ms | after: excerpt chars, names, ms |\n\
             |---|---|---|---|---|---|---|---|---|---|---|\n",
        );
        let mut attributes = String::from("\n## Attributes of the focused element\n");
        let workspace = NSWorkspace::sharedWorkspace();
        for running in workspace.runningApplications().iter() {
            let pid = running.processIdentifier();
            let bundle_id = running
                .bundleIdentifier()
                .map(|id| id.to_string())
                .unwrap_or_default();
            if running.activationPolicy() != NSApplicationActivationPolicy::Regular
                || pid <= 0
                || bundle_id.to_lowercase().contains("murmur")
                || is_password_app(&bundle_id)
            {
                continue;
            }
            let Some(app) = Element::application(pid) else {
                continue;
            };
            let focused = app.element("AXFocusedUIElement");
            let role = focused.as_ref().map_or("no focus".to_string(), |focused| {
                format!(
                    "{}/{}",
                    focused.string("AXRole").unwrap_or_default(),
                    focused.string("AXSubrole").unwrap_or_default()
                )
            });
            let secure = focused.as_ref().is_some_and(Element::is_secure);
            let readable = focused.as_ref().filter(|_| !secure);
            let selection = readable.and_then(|focused| focused.range("AXSelectedTextRange"));
            let ranged = readable.zip(selection).and_then(|(focused, selection)| {
                let count = FocusedText::char_count(focused)?;
                let end = (selection.start + selection.len + 2 * MAX_AFTER_CHARS + 2).min(count);
                let start = selection
                    .start
                    .saturating_sub(2 * MAX_BEFORE_CHARS + 2)
                    .min(end);
                focused.string_for_range(Utf16Range {
                    start,
                    len: end - start,
                })
            });
            let value = readable
                .filter(|_| selection.is_some())
                .and_then(|focused| focused.string("AXValue"));
            let selected = readable.and_then(|focused| focused.string("AXSelectedText"));
            let visible = readable
                .and_then(|focused| focused.range("AXVisibleCharacterRange"))
                .map(|range| range.len.to_string());
            let web_area = readable.and_then(|focused| {
                let mut element = focused.clone();
                for _ in 0..30 {
                    if element.string("AXRole").as_deref() == Some("AXWebArea") {
                        return Some(element);
                    }
                    element = element.element("AXParent")?;
                }
                None
            });
            let web_area = web_area.map_or("-".to_string(), |web_area| {
                let started = Instant::now();
                let text = window_text(web_area, started);
                format!(
                    "{}, {}",
                    text.chars().count(),
                    started.elapsed().as_millis()
                )
            });

            let started = Instant::now();
            let window = app
                .element("AXFocusedWindow")
                .map(|window| window_text(window, started));
            let before = window.map_or("no window".to_string(), |text| {
                let elapsed = started.elapsed().as_millis();
                let names = names_in(None, &text).len();
                format!("{}, {names}, {elapsed}", text.chars().count())
            });

            let started = Instant::now();
            let all = ContextReads {
                names: true,
                field: true,
            };
            let context = read_app(all, pid, &app, started);
            let elapsed = started.elapsed().as_millis();
            let excerpt = context.field.as_ref().map_or(0, |field| {
                field.before.chars().count() + field.after.chars().count()
            });
            let after = format!("{excerpt}, {}, {elapsed}", context.names.len());
            let split = context.field.as_ref().map_or("0/0".to_string(), |field| {
                format!(
                    "{}/{}",
                    field.before.chars().count(),
                    field.after.chars().count()
                )
            });
            let worked: Vec<&str> = [
                ("selection", selection.is_some()),
                ("AXStringForRange", ranged.is_some()),
                ("AXValue", value.is_some()),
                ("AXSelectedText", selected.is_some()),
                ("AXVisibleCharacterRange", visible.is_some()),
                ("web area", web_area != "-"),
            ]
            .into_iter()
            .filter_map(|(read, ok)| ok.then_some(read))
            .collect();
            let app_name = running
                .localizedName()
                .map_or_else(|| bundle_id.clone(), |name| name.to_string());
            writeln!(
                main,
                "| {app_name} | {before} | {split}, {}, {elapsed} | {role}{}; worked: {} |",
                context.names.len(),
                if secure { ", secure" } else { "" },
                if worked.is_empty() {
                    "none".to_string()
                } else {
                    worked.join(", ")
                }
            )
            .unwrap();

            writeln!(
                report,
                "| {bundle_id} | {role} | {secure} | {} | {} | {} | {} | {} | {web_area} | {before} | {after} |",
                selection.map_or("-".to_string(), |selection| format!("len {}", selection.len)),
                chars(ranged),
                chars(value),
                chars(selected),
                visible.unwrap_or_else(|| "-".to_string()),
            )
            .unwrap();
            if let Some(focused) = &focused {
                writeln!(
                    attributes,
                    "\n- {bundle_id}: {}; parameterized: {}",
                    focused.attribute_names(false).join(", "),
                    focused.attribute_names(true).join(", ")
                )
                .unwrap();
            }
        }
        main.push_str(&report);
        main.push_str(&attributes);
        std::fs::write(&out, &main).unwrap();
        println!("{main}");
    }

    #[test]
    fn names_in_the_excerpt_come_before_the_window_names() {
        let excerpt = field("Dear Siobhán and Priya, ", "with Ingrid");
        assert_eq!(
            names_in(excerpt.as_ref(), "meet Tobias and Siobhán today"),
            names(&["Siobhán", "Priya", "Ingrid", "Tobias"])
        );
    }

    /// A window's tree; reading a node's text takes `delay`, as in a slow app.
    #[derive(Clone, Default)]
    struct Node {
        text: &'static str,
        secure: bool,
        delay: Duration,
        children: Vec<Node>,
    }

    impl WindowNode for Node {
        fn is_secure(&self) -> bool {
            self.secure
        }

        fn texts(&self) -> Vec<String> {
            std::thread::sleep(self.delay);
            vec![self.text.to_string()]
        }

        fn children(&self) -> Vec<Self> {
            self.children.clone()
        }
    }

    fn node(text: &'static str, children: Vec<Node>) -> Node {
        Node {
            text,
            children,
            ..Node::default()
        }
    }

    #[test]
    fn the_window_is_read_breadth_first_without_secure_fields() {
        let window = node(
            "Draft",
            vec![
                node("To", vec![node("Ingrid", vec![])]),
                Node {
                    secure: true,
                    ..node("hunter2", vec![node("inside the password field", vec![])])
                },
                node("Subject", vec![]),
            ],
        );
        assert_eq!(
            window_text(window, Instant::now()),
            "Draft\nTo\nSubject\nIngrid\n"
        );
    }

    #[test]
    fn the_window_read_stops_after_the_node_limit() {
        let window = node("Window", vec![node("Item", vec![]); 2 * MAX_NODES]);
        let text = window_text(window, Instant::now());
        assert_eq!(text.lines().count(), MAX_NODES);
    }

    #[test]
    fn the_window_read_stops_when_the_budget_is_spent() {
        let slow = Node {
            delay: Duration::from_millis(50),
            ..node("Item", vec![])
        };
        let window = node("Window", vec![slow; 40]);
        let started = Instant::now();
        let text = window_text(window, started);
        assert!(started.elapsed() < READ_BUDGET + Duration::from_millis(150));
        assert!(text.lines().count() < 20, "{} lines", text.lines().count());
    }

    /// The production reads on real Win32 controls, in windows this test
    /// opens, so no foreground window is needed.
    #[cfg(target_os = "windows")]
    mod uia {
        use super::*;
        use crate::field_excerpt::{cut, read_around_selection, Utf16Range};
        use crate::uia::test_window::{child, TestWindow, PLAIN};
        use crate::uia::{Automation, Element};
        use windows::core::w;
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::System::LibraryLoader::LoadLibraryW;
        use windows::Win32::UI::WindowsAndMessaging::{
            SendMessageW, ES_AUTOHSCROLL, ES_PASSWORD, WINDOW_STYLE, WS_BORDER,
        };

        const EM_SETSEL: u32 = 0x00B1;
        /// The rich edit control of Msftedit.dll, as in WordPad and Windows
        /// 11's Notepad.
        const RICH_EDIT: &str = "RICHEDIT50W";

        fn later() -> Instant {
            Instant::now() + Duration::from_secs(60)
        }

        /// About 1 400 chars, so each side of the middle is longer than the
        /// excerpt.
        fn prose() -> String {
            "Dear Ingrid, the report from Tobias is ready for review. ".repeat(24)
        }

        /// A window holding one field of `class` with `text` and the
        /// selection `selection`, and a label.
        fn field_window(
            class: &'static str,
            style: WINDOW_STYLE,
            text: String,
            selection: Utf16Range,
        ) -> TestWindow {
            if class == RICH_EDIT {
                // SAFETY: loads a system library that registers the class.
                unsafe { LoadLibraryW(w!("Msftedit.dll")) }.expect("Msftedit.dll");
            }
            TestWindow::open(move |window| {
                let field = child(window, class, &text, WS_BORDER | style);
                // SAFETY: `field` is a live window of this thread.
                unsafe {
                    SendMessageW(
                        field,
                        EM_SETSEL,
                        Some(WPARAM(selection.start)),
                        Some(LPARAM((selection.start + selection.len) as isize)),
                    )
                };
                child(
                    window,
                    "STATIC",
                    "Lunch with Amaya",
                    WINDOW_STYLE::default(),
                );
                vec![field.0 as isize]
            })
        }

        fn element(automation: &Automation, window: isize) -> Element {
            automation.window(window).expect("element of the window")
        }

        #[test]
        fn the_excerpt_matches_the_one_cut_from_the_whole_field() {
            let automation = Automation::new().expect("UI Automation client");
            let text = prose();
            for class in ["EDIT", RICH_EDIT] {
                for selection in [
                    Utf16Range { start: 700, len: 0 },
                    Utf16Range {
                        start: 700,
                        len: 12,
                    },
                ] {
                    let window = field_window(
                        class,
                        WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                        text.clone(),
                        selection,
                    );
                    let field = element(&automation, window.handles[0]);
                    let excerpt = read_around_selection(&field, later());
                    assert!(excerpt.is_some(), "{class} {selection:?}");
                    assert_eq!(excerpt, cut(&text, selection), "{class} {selection:?}");
                }
            }
        }

        #[test]
        fn a_password_field_is_never_read() {
            let automation = Automation::new().expect("UI Automation client");
            let window = field_window(
                "EDIT",
                WINDOW_STYLE(ES_PASSWORD as u32),
                "hunter2 Corvina".to_string(),
                Utf16Range { start: 7, len: 0 },
            );
            let field = element(&automation, window.handles[0]);
            assert!(field.is_secure());
            assert_eq!(read_around_selection(&field, later()), None);
            let root = element(&automation, window.window);
            let text = window_text(
                UiaNode {
                    automation: &automation,
                    element: root,
                },
                Instant::now(),
            );
            assert!(text.contains("Amaya"), "the label is read");
            assert!(!text.contains("hunter2") && !text.contains("Corvina"));
        }

        #[test]
        fn a_control_without_text_or_caret_gives_no_excerpt() {
            let automation = Automation::new().expect("UI Automation client");
            let window = TestWindow::open(|window| {
                vec![child(window, "BUTTON", "Send", WINDOW_STYLE::default()).0 as isize]
            });
            let button = element(&automation, window.handles[0]);
            assert_eq!(read_around_selection(&button, later()), None);
        }

        #[test]
        fn names_come_from_the_field_and_the_rest_of_the_window() {
            let automation = Automation::new().expect("UI Automation client");
            let window = field_window(
                RICH_EDIT,
                WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                prose(),
                Utf16Range { start: 700, len: 0 },
            );
            let all = ContextReads {
                names: true,
                field: true,
            };
            let started = Instant::now();
            let context = read_window(
                all,
                || {
                    let field = element(&automation, window.handles[0]);
                    read_around_selection(&field, started + FIELD_BUDGET)
                },
                || {
                    let root = element(&automation, window.window);
                    window_text(
                        UiaNode {
                            automation: &automation,
                            element: root,
                        },
                        started,
                    )
                },
            );
            assert!(context.field.is_some());
            for name in ["Ingrid", "Tobias", "Amaya"] {
                assert!(context.names.contains(&name.to_string()), "{name}");
            }
        }

        #[test]
        fn a_window_of_many_controls_is_read_within_the_budget() {
            let automation = Automation::new().expect("UI Automation client");
            let window = TestWindow::open(|window| {
                for _ in 0..40 {
                    let group: HWND = child(window, PLAIN, "", WINDOW_STYLE::default());
                    for item in 0..40 {
                        child(
                            group,
                            "STATIC",
                            &format!("Item {item}"),
                            WINDOW_STYLE::default(),
                        );
                    }
                }
                Vec::new()
            });
            let root = element(&automation, window.window);
            let started = Instant::now();
            let text = window_text(
                UiaNode {
                    automation: &automation,
                    element: root,
                },
                started,
            );
            let elapsed = started.elapsed();
            assert!(text.contains("Item 0"));
            assert!(text.lines().count() < MAX_NODES, "{}", text.lines().count());
            // One call may take its whole timeout after the last check.
            assert!(
                elapsed < READ_BUDGET + Duration::from_millis(300),
                "{elapsed:?}"
            );
        }
    }
}
