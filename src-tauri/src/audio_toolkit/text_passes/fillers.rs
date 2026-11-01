use super::words::{spoken_case, Words};
use super::PassLanguage;

const EN: &[&str] = &["um", "umm", "uh", "uhh", "erm", "er"];
const FR: &[&str] = &["euh", "euhh", "heu", "hum"];

/// Drops hesitation words, with the comma or ellipsis that follows them.
pub(super) fn remove(words: &mut Words, language: PassLanguage) {
    let fillers = match language {
        PassLanguage::En => EN,
        PassLanguage::Fr => FR,
    };
    let mut index = 0;
    while index < words.items.len() {
        let word = &words.items[index].word;
        let is_filler = spoken_case(word) && fillers.contains(&word.to_lowercase().as_str());
        if !is_filler {
            index += 1;
            continue;
        }
        let capital =
            words.starts_sentence(index) && word.chars().next().is_some_and(char::is_uppercase);
        words.splice(index, index + 1, join);
        if capital {
            words.capitalize(index);
        }
    }
}

fn join(before: &str, after: &str, has_next: bool) -> String {
    let after = after.trim_start_matches([' ', '\t']);
    let after = after
        .strip_prefix("...")
        .or_else(|| after.strip_prefix(['…', ',']))
        .unwrap_or(after)
        .trim_start_matches([' ', '\t']);
    // A filler that is its own sentence ("Um." or "Hello. Um. So") takes its
    // punctuation with it.
    if before.is_empty() || before.trim_end().ends_with(['.', '!', '?', '…']) {
        let after = after.trim_start_matches(['.', '!', '?', ';', ':', ',', ' ', '\t']);
        return format!("{before}{after}");
    }
    if after.starts_with(['.', '!', '?', ';', ':', ',']) || (!has_next && after.is_empty()) {
        return format!("{}{after}", before.trim_end_matches([' ', '\t']));
    }
    if before.ends_with(char::is_whitespace) || after.starts_with(char::is_whitespace) {
        return format!("{before}{after}");
    }
    format!("{before} {after}")
}
