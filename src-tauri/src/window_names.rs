//! What the window the user dictates into offers the cleanup step: proper
//! names visible in it, as spelling hints, and a short excerpt of the
//! focused field around the insertion point. Both are read through
//! Accessibility on a background thread when dictation starts, reduced
//! locally, and dropped after use: only the names and the excerpt leave
//! this module, and nothing is stored or logged.

use crate::accessibility::Element;
use crate::field_excerpt::{read_field, FieldExcerpt, FocusedText, Utf16Range};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

fn read_frontmost_window(reads: ContextReads) -> WindowContext {
    let started = Instant::now();
    crate::accessibility::frontmost_app()
        .map(|(pid, bundle_id, app)| read_app(reads, pid, &bundle_id, &app, started))
        .unwrap_or_default()
}

/// The focused field first, then the focused window with what is left of
/// the budget that began at `started`.
fn read_app(
    reads: ContextReads,
    pid: i32,
    bundle_id: &str,
    app: &Element,
    started: Instant,
) -> WindowContext {
    read_window(
        reads,
        bundle_id,
        || {
            crate::accessibility::focus_in(pid, app)
                .and_then(|focused| read_excerpt(&focused, started + FIELD_BUDGET))
        },
        || {
            app.element("AXFocusedWindow")
                .map(|window| window_text(window, started))
                .unwrap_or_default()
        },
    )
}

fn read_excerpt(focused: &impl FocusedText, deadline: Instant) -> Option<FieldExcerpt> {
    read_field(focused, deadline, shows_secret)
}

/// Runs only the reads `reads` asks for. Without the window text, a code
/// app's other panes are not checked for assignments; the excerpt still is.
fn read_window(
    reads: ContextReads,
    bundle_id: &str,
    field: impl FnOnce() -> Option<FieldExcerpt>,
    window_text: impl FnOnce() -> String,
) -> WindowContext {
    let field = if reads.field { field() } else { None };
    let text = if reads.names {
        window_text()
    } else {
        String::new()
    };
    let context = window_context(bundle_id, &text, field);
    WindowContext {
        names: if reads.names {
            context.names
        } else {
            Vec::new()
        },
        field: context.field,
    }
}

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

/// The text of `window` and its descendants, breadth first, until the read
/// budget that started at `started` runs out.
fn window_text(window: Element, started: Instant) -> String {
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
    text
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
/// file or a shell session, so it gives no names and no excerpt at all.
/// Elsewhere an excerpt with a line that assigns a value, mentions a
/// password, token, secret, key, PIN or door code (in English or French),
/// holds a credential as a shell shows one, or holds a key or token with no
/// label, is dropped. Names in the excerpt come
/// first: they are next to what the user is writing.
fn window_context(
    bundle_id: &str,
    window_text: &str,
    field: Option<FieldExcerpt>,
) -> WindowContext {
    let excerpt = field
        .as_ref()
        .map(|field| format!("{}\n{}", field.before, field.after))
        .unwrap_or_default();
    let assigns = excerpt
        .lines()
        .chain(window_text.lines())
        .any(is_assignment);
    if is_code_app(bundle_id) && assigns {
        return WindowContext::default();
    }
    let mut names = candidate_names(&excerpt);
    for name in candidate_names(window_text) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.truncate(MAX_NAMES);
    WindowContext {
        names,
        field: field.filter(|_| !shows_secret(&excerpt)),
    }
}

/// A line of `text` assigns a value, also after a shell prompt, mentions a
/// secret, or holds a key.
fn shows_secret(text: &str) -> bool {
    text.lines().any(|line| {
        is_assignment(line)
            || line
                .split_whitespace()
                .any(|token| is_assignment(token) || holds_key(token))
            || mentions_secret(line)
    })
}

/// How the API keys and tokens of common services begin.
const KEY_PREFIXES: [&str; 11] = [
    "sk-",
    "sk_live_",
    "ghp_",
    "gho_",
    "github_pat_",
    "glpat-",
    "xoxa-",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "AIza",
];

/// A token, or a part of a URL or of `name=value` in it, that is a service's
/// key or a long generated one, with or without a label.
fn holds_key(token: &str) -> bool {
    token
        .split(['=', ':', '/', '?', '&', '#', '@'])
        .map(trim_word)
        .any(|part| {
            KEY_PREFIXES.iter().any(|prefix| {
                part.strip_prefix(prefix)
                    .is_some_and(|rest| rest.chars().count() >= 8)
            }) || (part.chars().count() >= LONG_TOKEN_CHARS && looks_generated(part))
        })
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

/// English and French labels next to a secret, lowercase and without
/// accents, French plurals spelled out, and the header that carries one in
/// a shell.
const SECRET_WORDS: [&str; 32] = [
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
    "authorization:",
    "bearer ",
    "mot de passe",
    "mots de passe",
    "mdp",
    "cle api",
    "cles api",
    "cle d'api",
    "cles d'api",
    "cle de l'api",
    "cles de l'api",
    "cle privee",
    "cles privees",
    "jeton",
    "identifiant",
    "code d'acces",
    "codes d'acces",
    "code pin",
    "codes pin",
    "digicode",
];

/// Short labels that name a secret only right before its value, as in
/// `PIN: 1234` or `OpenAI key: sk-…`: "the key point" holds none.
const VALUE_LABELS: [&str; 13] = [
    "key", "keys", "pin", "pass", "pwd", "pw", "login", "cle", "cles", "code", "codes", "wifi",
    "wi-fi",
];

/// Words that name a secret when a value with a digit follows within two
/// words, as in `PIN 1234` or "le code est 4512".
const NUMBER_LABELS: [&str; 3] = ["pin", "code", "codes"];

/// Codes next to these words are no secret: "Code postal", "Zip code".
const PUBLIC_CODES: [&str; 4] = ["postal", "zip", "promo", "source"];

/// French names a secret first and what it opens after it, within three
/// words: "Clé Wi-Fi", "Clé de sécurité", "Code de la porte".
const FRENCH_SECRET_HEADS: [&str; 4] = ["cle", "cles", "code", "codes"];
const FRENCH_SECRET_OBJECTS: [&str; 18] = [
    "wifi",
    "wi-fi",
    "wpa",
    "securite",
    "porte",
    "immeuble",
    "entree",
    "interphone",
    "batiment",
    "alarme",
    "portail",
    "garage",
    "parking",
    "cadenas",
    "coffre",
    "boite",
    "cb",
    "carte",
];

/// English names what a code opens first: "Door code", "Alarm code".
const ENGLISH_CODE_OBJECTS: [&str; 6] = ["door", "gate", "alarm", "access", "lockbox", "security"];

fn mentions_secret(line: &str) -> bool {
    let folded = folded(line);
    SECRET_WORDS.iter().any(|word| folded.contains(word))
        || names_a_secret(&folded)
        || has_url_credential(line)
        || has_user_option(line)
}

/// A `VALUE_LABELS` word or "PIN code" right before a colon, a
/// `NUMBER_LABELS` word before a digit, a French secret head or an English
/// code object, in whole words of the `folded` line. A code next to a
/// `PUBLIC_CODES` word counts for none of them.
fn names_a_secret(folded: &str) -> bool {
    fn words(text: &str) -> Vec<&str> {
        text.split(|c: char| c.is_whitespace() || c == '\'')
            .map(trim_word)
            .filter(|word| !word.is_empty())
            .collect()
    }
    let is_label = |label: &str| match words(label).as_slice() {
        [.., "pin", "code"] => true,
        [.., before, "code" | "codes"] if PUBLIC_CODES.contains(before) => false,
        [.., last] => VALUE_LABELS.contains(last),
        [] => false,
    };
    let labelled = folded
        .match_indices(':')
        .any(|(colon, _)| is_label(&folded[..colon]));
    let words = words(folded);
    let numbered = |at: usize| {
        !words[at.saturating_sub(1)..(at + 2).min(words.len())]
            .iter()
            .any(|near| PUBLIC_CODES.contains(near))
            && words[at + 1..]
                .iter()
                .take(2)
                .any(|next| next.chars().any(|c| c.is_ascii_digit()))
    };
    labelled
        || words.iter().enumerate().any(|(at, word)| {
            (NUMBER_LABELS.contains(word) && numbered(at))
                || (FRENCH_SECRET_HEADS.contains(word)
                    && words[at + 1..]
                        .iter()
                        .take(3)
                        .any(|next| FRENCH_SECRET_OBJECTS.contains(next)))
        })
        || words.windows(2).any(|pair| {
            ENGLISH_CODE_OBJECTS.contains(&pair[0]) && matches!(pair[1], "code" | "codes")
        })
}

/// Lowercase, without French accents, and with `’` as `'`.
fn folded(line: &str) -> String {
    line.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '’' => '\'',
            c => c,
        })
        .collect()
}

/// A URL that holds a password, as in `postgres://user:password@host`, or a
/// token as its user, as in `https://ghp_…@github.com`.
fn has_url_credential(line: &str) -> bool {
    line.match_indices("://").any(|(at, _)| {
        let authority = line[at + 3..]
            .split(['/', '?', '#', ' ', '"', '\''])
            .next()
            .unwrap_or_default();
        authority
            .rsplit_once('@')
            .is_some_and(|(user, _)| user.contains(':') || looks_generated(user))
    })
}

/// A user and password given to a command, as in `curl -u user:password`.
fn has_user_option(line: &str) -> bool {
    let mut words = line.split_whitespace();
    while let Some(word) = words.next() {
        if matches!(word, "-u" | "--user")
            && words.next().is_some_and(|user| {
                user.split_once(':')
                    .is_some_and(|(_, pass)| !pass.is_empty())
            })
        {
            return true;
        }
    }
    false
}

/// Words that look like names: capitalised but not merely sentence case,
/// never seen in lowercase in the same text, plus the name parts of email
/// addresses and @handles. Most frequent first, at most 50. Lines that
/// assign a value or mention a password, token, secret or API key are
/// skipped, and so is the line after them, where a label's value sits, and
/// every entry of the list they head, up to a sentence or a blank line
/// other than one right under the label.
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
    let mut in_secret_list = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            in_secret_list &= after_secret_label;
            continue;
        }
        let label = mentions_secret(line);
        let entry = in_secret_list && is_list_entry(line);
        let sensitive = after_secret_label || entry || is_assignment(line) || label;
        // An entry such as `Wi-Fi : Soleil2024` holds its own value.
        after_secret_label = label && !entry;
        in_secret_list = label || (in_secret_list && sensitive);
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

/// A line of a list such as `Bureau - Corvina`, `Bureau : Corvina`,
/// `Bureau Corvina`, `- Bureau Corvina` or `2. Bureau Corvina`: an entry
/// and its value, rather than a sentence.
fn is_list_entry(line: &str) -> bool {
    let line = line.trim();
    let digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
    let after_marker = if digits.len() < line.len() {
        digits.strip_prefix(['.', ')'])
    } else {
        line.strip_prefix(['-', '*', '•', '–', '—'])
    };
    after_marker.is_some_and(|rest| rest.starts_with(char::is_whitespace))
        || [" - ", " – ", " — ", ":", "\t", " | "]
            .iter()
            .any(|separator| line.contains(separator))
        || (line.split_whitespace().count() <= 3 && line.starts_with(char::is_uppercase))
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
                    "com.apple.Notes",
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
    fn terminals_and_editors_showing_assignments_give_no_names() {
        let text = "DATABASE_URL=postgres://localhost\nwritten by Ingrid";
        let context = |bundle_id, text| window_context(bundle_id, text, None).names;
        assert!(context("com.apple.Terminal", text).is_empty());
        assert!(context("com.jetbrains.intellij", text).is_empty());
        assert_eq!(context("com.apple.Notes", text), names(&["Ingrid"]));
        assert_eq!(
            context("com.microsoft.VSCode", "written by Ingrid"),
            names(&["Ingrid"])
        );
    }

    #[test]
    fn a_code_app_showing_assignments_gives_no_excerpt_either() {
        let window = "DATABASE_URL=postgres://localhost\nwritten by Ingrid";
        let excerpt = field("thanks to Ingrid for ", "");
        assert_eq!(
            window_context("com.apple.Terminal", window, excerpt.clone()),
            WindowContext::default()
        );
        assert_eq!(
            window_context("com.apple.Notes", window, excerpt.clone()).field,
            excerpt
        );
    }

    /// Every excerpt the secret filter was checked on: the app, its window
    /// text, the text before and after the cursor, and whether the excerpt
    /// may be sent. Add rows; never change one to make a fix pass.
    const EXCERPT_CORPUS: &[(&str, &str, &str, &str, bool)] = &[
        // English labels and assignments.
        (
            "com.apple.Notes",
            "",
            "Wi-Fi password: Bluebird\nsee you ",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "Hi Ingrid, ",
            " here is my API key for staging",
            false,
        ),
        ("com.apple.Notes", "", "STRIPE_KEY=Moonlight\n", "", false),
        (
            "com.apple.Notes",
            "",
            "export ADMIN = Corvina\nthen ",
            "run it",
            false,
        ),
        ("com.apple.Notes", "", "the token is ", "", false),
        (
            "com.apple.Notes",
            "",
            "Thanks for the draft, Priya. ",
            "",
            true,
        ),
        // A code app showing an assignment sends nothing; elsewhere the
        // window's assignment is not the excerpt's.
        (
            "com.apple.Terminal",
            "DATABASE_URL=postgres://localhost\nwritten by Ingrid",
            "thanks to Ingrid for ",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "DATABASE_URL=postgres://localhost\nwritten by Ingrid",
            "thanks to Ingrid for ",
            "",
            true,
        ),
        // French labels (round 44).
        (
            "com.apple.Notes",
            "",
            "Le mot de passe du Wi-Fi est Soleil2024\n",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "mdp : Soleil2024\nà demain ",
            "",
            false,
        ),
        ("com.apple.Mail", "", "clé API : sk-abc123\n", "", false),
        (
            "com.apple.Mail",
            "",
            "Voici la clé d'API de la préprod ",
            "",
            false,
        ),
        (
            "com.apple.Mail",
            "",
            "Voici la clé d’API de la préprod ",
            "",
            false,
        ),
        ("com.apple.Notes", "", "cle api : sk-abc123\n", "", false),
        (
            "com.apple.Notes",
            "",
            "Ton jeton d'accès expire demain, ",
            "",
            false,
        ),
        ("com.apple.Notes", "", "Identifiant : jdupont\n", "", false),
        (
            "com.apple.Notes",
            "",
            "Code d'accès de l'immeuble : 4589B\n",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "Code PIN de la carte : 1234\n",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "Merci pour la relecture, Priya. ",
            "",
            true,
        ),
        (
            "com.apple.Notes",
            "",
            "Le mot de la fin revient à Ingrid. ",
            "",
            true,
        ),
        // Credentials in a terminal's scrollback (round 44).
        (
            "com.apple.Terminal",
            "",
            "$ curl -H \"Authorization: Bearer sk-live-abc\" https://api.example.com\n$ ",
            "",
            false,
        ),
        (
            "com.googlecode.iterm2",
            "",
            "$ psql postgres://admin:S3cret@db.example.com/app\n$ ",
            "",
            false,
        ),
        (
            "com.googlecode.iterm2",
            "",
            "$ git clone ssh://git@github.com/example/app.git\n$ ",
            "",
            true,
        ),
        (
            "com.apple.Terminal",
            "",
            "$ open https://example.com/docs?page=2\n$ ",
            "",
            true,
        ),
        // French plurals and variants (round 45).
        (
            "com.apple.Notes",
            "",
            "Mots de passe :\nWi-Fi : Soleil2024\n",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "Les clés API : sk-abc123\n",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "Clés privées du serveur : Corvina\n",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "Codes d'accès : 4589B et 7781A\n",
            "",
            false,
        ),
        ("com.apple.Notes", "", "Codes PIN : 1234, 5678\n", "", false),
        (
            "com.apple.Mail",
            "",
            "Voici la clé de l'API : sk-abc123\n",
            "",
            false,
        ),
        ("com.apple.Notes", "", "Digicode : 4589B\n", "", false),
        // A shell prompt before the command (round 45).
        (
            "com.apple.Terminal",
            "",
            "$ export DB_PASS=hunter2\n$ ",
            "",
            false,
        ),
        (
            "com.googlecode.iterm2",
            "",
            "dev@Mac app % export STRIPE_KEY=sk_live_abc\ndev@Mac app % ",
            "",
            false,
        ),
        (
            "com.apple.Terminal",
            "",
            "$ curl -u admin:S3cret https://api.example.com\n$ ",
            "",
            false,
        ),
        (
            "com.apple.Terminal",
            "",
            "$ git clone https://ghp_x7Kq2mN9vR4bL1tW@github.com/example/app.git\n$ ",
            "",
            false,
        ),
        (
            "com.apple.Terminal",
            "",
            "$ git clone https://dev@example.com/app.git\n$ ",
            "",
            true,
        ),
        // The 300-character cut goes through the label (round 45).
        (
            "com.apple.Notes",
            "",
            "Mot de passe :\nSoleil2024\nla suite. la suite. la suite. la suite. la suite. \
             la suite. la suite. la suite. la suite. la suite. la suite. la suite. \
             la suite. la suite. la suite. la suite. la suite. la suite. la suite. \
             la suite. la suite. la suite. la suite. la suite. la suite. la suite. \
             la suite. la suite. ",
            "",
            false,
        ),
        // Labels with no listed word, and keys with no label (round 46).
        ("com.apple.Notes", "", "Clé Wi-Fi : Soleil2024\n", "", false),
        (
            "com.apple.Notes",
            "",
            "Clé de sécurité : Soleil2024\n",
            "",
            false,
        ),
        ("com.apple.Notes", "", "Code porte : 4589B\n", "", false),
        (
            "com.apple.Notes",
            "",
            "Le code de la porte est 4589B, ",
            "",
            false,
        ),
        ("com.apple.Notes", "", "PIN: 1234\n", "", false),
        ("com.apple.Notes", "", "PIN code: 1234\n", "", false),
        (
            "com.apple.Notes",
            "",
            "Login: jdoe / pass: hunter2\n",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "OpenAI key: sk-proj-4fT9xQ2mN7vR1bL8\n",
            "",
            false,
        ),
        (
            "com.tinyspeck.slackmacgap",
            "",
            "here you go sk-proj-4fT9xQ2mN7vR1bL8 thanks ",
            "",
            false,
        ),
        (
            "com.tinyspeck.slackmacgap",
            "",
            "ghp_x7Kq2mN9vR4bL1tW8cZ3dF6gH0jK5pS2uY4 ",
            "",
            false,
        ),
        (
            "com.apple.Mail",
            "",
            "the id is AKIAIOSFODNN7EXAMPLE ",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "https://maps.googleapis.com/api?key=AIzaSyD4fT9xQ2mN7vR1bL8cW3zK5pS2uY4hJ0 ",
            "",
            false,
        ),
        (
            "com.apple.Notes",
            "",
            "Q3 report: w8Kd2LpQ9xVz3RtY7mNc4BfH1jGs ",
            "",
            false,
        ),
        // The label of a list longer than the excerpt (round 46).
        (
            "com.apple.Notes",
            "",
            "Mots de passe\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n",
            "",
            false,
        ),
        // The same words in ordinary text (round 46).
        (
            "com.apple.Notes",
            "",
            "The key point is the launch date. ",
            "",
            true,
        ),
        (
            "com.apple.Notes",
            "",
            "Key takeaways: ship on Monday. ",
            "",
            true,
        ),
        (
            "com.apple.Notes",
            "",
            "La clé du succès, c'est la régularité. ",
            "",
            true,
        ),
        ("com.apple.Notes", "", "Code postal : 75001\n", "", true),
        (
            "com.apple.Notes",
            "",
            "I'll pass on dinner, pin the tab and ",
            "",
            true,
        ),
        (
            "com.apple.Notes",
            "",
            "The login page is broken again. ",
            "",
            true,
        ),
        (
            "com.apple.Notes",
            "",
            "Rendez-vous à 14h30 le 2024-10-23, réf. 648e96d6. ",
            "",
            true,
        ),
        (
            "com.apple.Mail",
            "",
            "Écris à jean-baptiste.delacroix@exemple-societe.fr pour ",
            "",
            true,
        ),
        ("com.apple.Notes", "", "We use sk-learn and ", "", true),
        (
            "com.apple.Notes",
            "",
            "Mots de passe\nBanque : Soleil2024\n\nLa réunion de lundi parle du budget. ",
            "",
            false,
        ),
        // Door codes, PINs and Wi-Fi keys with no listed label (round 47).
        ("com.apple.Notes", "", "Code : 4512B\n", "", false),
        ("com.apple.Notes", "", "Code: 1234\n", "", false),
        ("com.apple.Notes", "", "Le code est 4512, ", "", false),
        ("com.apple.Notes", "", "Door code: 4512\n", "", false),
        ("com.apple.Notes", "", "Gate code 4512\n", "", false),
        ("com.apple.Notes", "", "Alarm code: 1234\n", "", false),
        ("com.apple.Notes", "", "Access code: 482913\n", "", false),
        ("com.apple.Notes", "", "PIN 1234\n", "", false),
        ("com.apple.Messages", "", "My PIN is 1234, ", "", false),
        ("com.apple.Notes", "", "PIN number: 1234\n", "", false),
        ("com.apple.Notes", "", "Wifi : Soleil2024\n", "", false),
        ("com.apple.Mail", "", "Wi-Fi: Bluebird2024\n", "", false),
        (
            "com.apple.Notes",
            "",
            "Code interphone : 4512B\n",
            "",
            false,
        ),
        ("com.apple.Notes", "", "Code bâtiment : 4512\n", "", false),
        ("com.apple.Notes", "", "Code garage : 1234\n", "", false),
        ("com.apple.Notes", "", "Mon code PIN est 1234, ", "", false),
        ("com.apple.Notes", "", "Zip code: 94103\n", "", true),
        (
            "com.apple.Notes",
            "",
            "Code promo : BIENVENUE10\n",
            "",
            true,
        ),
        (
            "com.apple.Notes",
            "",
            "The code review is on Monday. ",
            "",
            true,
        ),
        // A blank line under the label of a list longer than the excerpt
        // (round 47).
        (
            "com.apple.Notes",
            "",
            "Mots de passe\n\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n\
             Banque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\nBanque : Soleil2024\n",
            "",
            false,
        ),
    ];

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
        // Values next to a secret label (rounds 44 and 45).
        (
            "Wi-Fi password: Bluebird\nshared with Corvina\nthanks to Ingrid",
            &["Ingrid"],
        ),
        ("API key\nZephyrine\nmeet Tobias", &["Tobias"]),
        (
            "STRIPE_KEY=Moonlight\nexport ADMIN = Corvina\nask Amaya",
            &["Amaya"],
        ),
        ("Mots de passe :\nWi-Fi : Soleil2024\nmerci Ingrid", &["Ingrid"]),
        // Every entry of a list under a secret label (round 46).
        (
            "Wi-Fi password\nMaison - Bluebird\nBureau - Corvina\nGîte - Soleil2024",
            &[],
        ),
        (
            "Mots de passe\nMaison Bluebird\nBureau Corvina\nGîte Soleil2024\n\nmerci Ingrid",
            &["Ingrid"],
        ),
        (
            "Clé Wi-Fi : Bluebird\nBureau : Corvina\nwith Tobias",
            &["Tobias"],
        ),
        (
            "Code porte : 4589B\nGarage – Corvina\nask Amaya",
            &["Amaya"],
        ),
        // Numbered and bulleted entries, and a blank line under the label
        // (round 47).
        (
            "Mots de passe\n1. Banque Soleil2024\n2. Netflix Lune2023\n3. Gmail Etoile99",
            &[],
        ),
        (
            "Mots de passe\n- Banque Soleil2024\n- Netflix Lune2023\n- Gmail Etoile99",
            &[],
        ),
        (
            "Passwords\n• Bank Bluebird7\n• Netflix Corvina\n• Gmail Zephyr42",
            &[],
        ),
        (
            "Mots de passe\n\nBanque Soleil2024\nNetflix Lune2023\n\nmerci Ingrid",
            &["Ingrid"],
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

    /// A field holding `0` with the cursor at UTF-16 unit `1`.
    struct Field(String, usize);

    impl FocusedText for Field {
        fn is_secure(&self) -> bool {
            false
        }
        fn selection(&self) -> Option<Utf16Range> {
            Some(Utf16Range {
                start: self.1,
                len: 0,
            })
        }
        fn char_count(&self) -> Option<usize> {
            Some(self.0.encode_utf16().count())
        }
        fn string_for_range(&self, range: Utf16Range) -> Option<String> {
            let units: Vec<u16> = self.0.encode_utf16().collect();
            units
                .get(range.start..range.start + range.len)
                .map(String::from_utf16_lossy)
        }
        fn value(&self) -> Option<String> {
            Some(self.0.clone())
        }
    }

    #[test]
    fn excerpt_secret_corpus() {
        let wrong: Vec<String> = EXCERPT_CORPUS
            .iter()
            .filter(|&&(app, window, before, after, kept)| {
                let text = format!("{before}{after}");
                let cursor = before.encode_utf16().count();
                let deadline = Instant::now() + Duration::from_secs(60);
                let excerpt = read_excerpt(&Field(text.clone(), cursor), deadline);
                let expected = if kept {
                    crate::field_excerpt::cut(
                        &text,
                        Utf16Range {
                            start: cursor,
                            len: 0,
                        },
                    )
                } else {
                    None
                };
                window_context(app, window, excerpt).field != expected
            })
            .map(|(app, _, before, after, kept)| format!("{app} {before:?} {after:?} kept: {kept}"))
            .collect();
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Measures every read on each regular app running, without bringing
    /// any to the front, and writes a markdown table. Only lengths, counts,
    /// roles and attribute names are recorded, never text.
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
                let names = window_context(&bundle_id, &text, None).names.len();
                format!("{}, {names}, {elapsed}", text.chars().count())
            });

            let started = Instant::now();
            let all = ContextReads {
                names: true,
                field: true,
            };
            let context = read_app(all, pid, &bundle_id, &app, started);
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
        let context = window_context(
            "com.apple.Mail",
            "meet Tobias and Siobhán today",
            field("Dear Siobhán and Priya, ", "with Ingrid"),
        );
        assert_eq!(
            context.names,
            names(&["Siobhán", "Priya", "Ingrid", "Tobias"])
        );
    }
}
