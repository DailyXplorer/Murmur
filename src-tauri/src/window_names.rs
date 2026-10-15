//! Proper names visible in the window the user dictates into, offered to the
//! cleanup step as spelling hints. The window text is read through
//! Accessibility on a background thread when dictation starts, reduced to
//! candidate names locally, and dropped after use: only the names leave
//! this module, and nothing is stored or logged.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_NAMES: usize = 50;
const MAX_NAME_CHARS: usize = 40;
const MAX_NODES: usize = 1_500;
const MAX_TEXT_CHARS: usize = 40_000;
const READ_BUDGET: Duration = Duration::from_millis(400);
/// Stopping must not wait on a slow app; late names are simply not used.
const COLLECT_WAIT: Duration = Duration::from_millis(50);

static PENDING: Mutex<Option<Receiver<Vec<String>>>> = Mutex::new(None);

/// Starts reading the frontmost window. Replaces any unclaimed capture.
pub fn capture() {
    let (sender, receiver) = mpsc::channel();
    if let Ok(mut pending) = PENDING.lock() {
        *pending = Some(receiver);
    }
    std::thread::spawn(move || {
        let names = read_frontmost_window()
            .map(|text| candidate_names(&text))
            .unwrap_or_default();
        let _ = sender.send(names);
    });
}

/// The names captured for the dictation that is ending, if ready.
pub fn collect() -> Vec<String> {
    let receiver = PENDING.lock().ok().and_then(|mut pending| pending.take());
    receiver
        .and_then(|receiver| receiver.recv_timeout(COLLECT_WAIT).ok())
        .unwrap_or_default()
}

fn read_frontmost_window() -> Option<String> {
    use crate::accessibility::frontmost_app;

    let started = Instant::now();
    let (_, app) = frontmost_app()?;
    let window = app.element("AXFocusedWindow")?;
    let mut text = String::new();
    let mut queue = std::collections::VecDeque::from([window]);
    let mut visited = 0;
    while let Some(element) = queue.pop_front() {
        visited += 1;
        if visited > MAX_NODES || text.len() > MAX_TEXT_CHARS || started.elapsed() > READ_BUDGET {
            break;
        }
        if element.is_secure() {
            continue;
        }
        for attribute in ["AXValue", "AXTitle", "AXDescription"] {
            if let Some(value) = element.string(attribute) {
                if !value.trim().is_empty() {
                    text.push_str(&value);
                    text.push('\n');
                }
            }
        }
        queue.extend(element.children());
    }
    Some(text)
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
}
