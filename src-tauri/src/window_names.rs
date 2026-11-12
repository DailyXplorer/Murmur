//! What the window the user dictates into offers the cleanup step: proper
//! names visible in it, as spelling hints, and a short excerpt of the
//! focused field around the insertion point. Both are read through
//! Accessibility on macOS, or UI Automation on Windows, on a background
//! thread when dictation starts, reduced locally, and dropped after use:
//! only the names and the excerpt leave this module, and nothing is stored
//! or logged.

use crate::field_excerpt::FieldExcerpt;
use crate::insights::categories::{categorize, AppCategory};
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
    /// The kind of app the window belongs to, read when a kind has a style.
    pub kind: Option<AppCategory>,
}

/// Which parts of the frontmost window a dictation reads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextReads {
    /// Names visible anywhere in the focused window.
    pub names: bool,
    /// The excerpt of the focused field around the insertion point.
    pub field: bool,
    /// The kind of app in front, as Insights sorts it.
    pub kind: bool,
}

static PENDING: Mutex<Option<Receiver<WindowContext>>> = Mutex::new(None);

/// Starts a dictation: reads what `reads` asks for from the frontmost
/// window, and always drops the context of an earlier dictation nobody
/// claimed, such as one cancelled with Escape.
pub fn capture(reads: ContextReads) {
    capture_with(reads, |reads| WindowContext {
        kind: reads.kind.then(frontmost_kind).flatten(),
        ..read_frontmost_window(reads)
    });
}

fn frontmost_kind() -> Option<AppCategory> {
    crate::insights::frontmost::current()
        .map(|target| categorize(&target.bundle_id, target.host.as_deref()))
}

fn capture_with(
    reads: ContextReads,
    read: impl FnOnce(ContextReads) -> WindowContext + Send + 'static,
) {
    let (sender, receiver) = mpsc::channel();
    let reads_any = reads.names || reads.field || reads.kind;
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
    WindowContext {
        names,
        field,
        kind: None,
    }
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
    window_text_until(window, || started.elapsed() > READ_BUDGET)
}

/// `window_text` with the budget as a question, asked before each element:
/// has the read budget run out?
fn window_text_until(window: impl WindowNode, mut spent: impl FnMut() -> bool) -> String {
    let mut text = String::new();
    let mut queue = VecDeque::from([window]);
    let mut visited = 0;
    while let Some(element) = queue.pop_front() {
        visited += 1;
        if visited > MAX_NODES || text.len() > MAX_TEXT_CHARS || spent() {
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
mod tests;
