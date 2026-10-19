//! Proper names visible in the window the user dictates into, offered to the
//! cleanup step as spelling hints. The window text is read through
//! Accessibility on a background thread when dictation starts, reduced to
//! candidate names locally, and dropped after use: only the names leave
//! this module, and nothing is stored or logged.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant;

const MAX_NAMES: usize = 50;
const MAX_NAME_CHARS: usize = 40;
/// Unbroken tokens this long are keys or hashes, not names.
const LONG_TOKEN_CHARS: usize = 20;
#[cfg(target_os = "macos")]
const MAX_NODES: usize = 1_500;
#[cfg(target_os = "macos")]
const MAX_TEXT_CHARS: usize = 40_000;
#[cfg(target_os = "macos")]
const READ_BUDGET: Duration = Duration::from_millis(400);
/// Stopping must not wait on a slow app; late names are simply not used.
const COLLECT_WAIT: Duration = Duration::from_millis(50);

static PENDING: Mutex<Option<Receiver<Vec<String>>>> = Mutex::new(None);

/// Starts a dictation: reads the frontmost window when `enabled`, and
/// always drops the names of an earlier dictation nobody claimed, such as
/// one cancelled with Escape.
pub fn capture(enabled: bool) {
    let (sender, receiver) = mpsc::channel();
    if let Ok(mut pending) = PENDING.lock() {
        *pending = enabled.then_some(receiver);
    }
    if !enabled {
        return;
    }
    std::thread::spawn(move || {
        let names = read_frontmost_window()
            .map(|(bundle_id, text)| names_in_window(&bundle_id, &text))
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

/// Reading another app's window text on Windows needs UI Automation, which
/// this build does not have; the setting is off there (see `settings`).
#[cfg(target_os = "windows")]
fn read_frontmost_window() -> Option<(String, String)> {
    None
}

/// The frontmost app's bundle identifier and the text of its focused window.
#[cfg(target_os = "macos")]
fn read_frontmost_window() -> Option<(String, String)> {
    use crate::accessibility::frontmost_app;

    let started = Instant::now();
    let (_, bundle_id, app) = frontmost_app()?;
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
    Some((bundle_id, text))
}

/// Terminals and code editors, where a `.env` file or a shell `export` may
/// be on screen.
const CODE_APPS: [&str; 18] = [
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "dev.warp.Warp-Stable",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "com.github.wez.wezterm",
    "com.mitchellh.ghostty",
    "co.zeit.hyper",
    "com.microsoft.VSCode",
    "com.microsoft.VSCodeInsiders",
    "com.vscodium",
    "com.todesktop.230313mzl4w4u92",
    "dev.zed.Zed",
    "com.sublimetext.4",
    "com.apple.dt.Xcode",
    "com.panic.Nova",
    "com.barebones.bbedit",
    "com.apple.TextEdit",
];
const CODE_APP_PREFIXES: [&str; 1] = ["com.jetbrains."];

fn is_code_app(bundle_id: &str) -> bool {
    CODE_APPS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(bundle_id))
        || CODE_APP_PREFIXES
            .iter()
            .any(|prefix| bundle_id.starts_with(prefix))
}

/// A terminal or editor with any `KEY=value` line may be showing a `.env`
/// file or a shell session, so it gives no names at all.
fn names_in_window(bundle_id: &str, text: &str) -> Vec<String> {
    if is_code_app(bundle_id) && text.lines().any(is_assignment) {
        return Vec::new();
    }
    candidate_names(text)
}

/// `KEY=value` or `export KEY=value`, as in a `.env` file or a shell.
fn is_assignment(line: &str) -> bool {
    let line = line.trim();
    let line = line.strip_prefix("export ").unwrap_or(line);
    let Some((key, value)) = line.split_once('=') else {
        return false;
    };
    let key = key.trim_end();
    key.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        && !value.trim().is_empty()
        && !value.starts_with('=')
}

const SECRET_WORDS: [&str; 12] = [
    "password",
    "passwd",
    "passcode",
    "passphrase",
    "token",
    "secret",
    "api key",
    "api_key",
    "api-key",
    "apikey",
    "private key",
    "credential",
];

fn mentions_secret(line: &str) -> bool {
    let line = line.to_lowercase();
    SECRET_WORDS.iter().any(|word| line.contains(word))
}

/// Words that look like names: capitalised but not merely sentence case,
/// never seen in lowercase in the same text, plus the name parts of email
/// addresses and @handles. Most frequent first, at most 50. Lines that
/// assign a value or mention a password, token, secret or API key are
/// skipped, and so is the line after them, where a label's value sits.
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

    let mut after_secret_label = false;
    for line in text.lines() {
        let sensitive = after_secret_label || is_assignment(line) || mentions_secret(line);
        if !line.trim().is_empty() {
            after_secret_label = mentions_secret(line);
        }
        if sensitive {
            continue;
        }
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

/// Passwords, API keys and hashes rather than words. Written names mix
/// letters and digits at most once ("GPT-4o", "iPhone15"), switch case at
/// most twice ("McDonald", "DeepSeek"), have vowels, and stay short unless
/// they are hyphenated words.
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
    fn generated_secrets_are_not_names() {
        for secret in [
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
                candidate_names(&format!("see {secret} here")).is_empty(),
                "accepted {secret}"
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

    #[test]
    fn values_next_to_secret_labels_are_skipped() {
        assert_eq!(
            candidate_names("Wi-Fi password: Bluebird\nshared with Corvina\nthanks to Ingrid"),
            vec!["Ingrid"]
        );
        assert_eq!(
            candidate_names("API key\nZephyrine\nmeet Tobias"),
            vec!["Tobias"]
        );
        assert_eq!(
            candidate_names("STRIPE_KEY=Moonlight\nexport ADMIN = Corvina\nask Amaya"),
            vec!["Amaya"]
        );
    }

    #[test]
    fn a_dictation_started_with_names_off_drops_an_unclaimed_capture() {
        let (sender, receiver) = mpsc::channel();
        sender.send(vec!["Ingrid".to_string()]).unwrap();
        *PENDING.lock().unwrap() = Some(receiver);
        capture(false);
        assert_eq!(collect(), Vec::<String>::new());
    }

    #[test]
    fn terminals_and_editors_showing_assignments_give_no_names() {
        let text = "DATABASE_URL=postgres://localhost\nwritten by Ingrid";
        assert!(names_in_window("com.apple.Terminal", text).is_empty());
        assert!(names_in_window("com.jetbrains.intellij", text).is_empty());
        assert_eq!(names_in_window("com.apple.Notes", text), vec!["Ingrid"]);
        assert_eq!(
            names_in_window("com.microsoft.VSCode", "written by Ingrid"),
            vec!["Ingrid"]
        );
    }
}
