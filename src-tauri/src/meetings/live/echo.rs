//! Without headphones the microphone hears the remote side too. A live
//! microphone sentence that mostly repeats what system audio said at the same
//! time is that echo, and is not shown. The final transcript filters echo on
//! its own, with the levels (`bleed/`).

use std::collections::HashSet;

use super::LiveEntry;
use crate::meetings::tokens::key;

/// How far apart in time an echo and its source may be.
pub const ECHO_WINDOW_MS: u64 = 1_500;
const ECHO_SHARE: f64 = 0.6;

fn keys(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(key)
        .filter(|key| !key.is_empty())
        .collect()
}

/// Whether `mic` repeats the `system` sentences said around it.
pub fn is_echo(mic: &LiveEntry, system: &[LiveEntry]) -> bool {
    let from = mic.start_ms.saturating_sub(ECHO_WINDOW_MS);
    let to = mic.end_ms + ECHO_WINDOW_MS;
    let heard: HashSet<String> = system
        .iter()
        .filter(|entry| entry.start_ms <= to && entry.end_ms >= from)
        .flat_map(|entry| keys(&entry.text))
        .collect();
    let words = keys(&mic.text);
    if words.is_empty() || heard.is_empty() {
        return false;
    }
    let shared = words.iter().filter(|word| heard.contains(*word)).count();
    shared as f64 >= ECHO_SHARE * words.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::live::LiveEntryKind;
    use crate::meetings::merge::Track as Source;

    fn entry(source: Source, text: &str, start_ms: u64, end_ms: u64) -> LiveEntry {
        LiveEntry {
            source,
            start_ms,
            end_ms,
            text: text.into(),
            kind: LiveEntryKind::Speech,
        }
    }

    #[test]
    fn mic_echo_of_system_sentence_is_hidden() {
        let system = [entry(
            Source::System,
            "The release moves to Thursday, after the review.",
            20_000,
            23_500,
        )];
        let echo = entry(
            Source::Mic,
            "release moves to Thursday after review",
            20_400,
            23_900,
        );
        assert!(is_echo(&echo, &system));
    }

    #[test]
    fn distinct_mic_sentence_is_kept() {
        let system = [entry(
            Source::System,
            "The release moves to Thursday, after the review.",
            20_000,
            23_500,
        )];
        let reply = entry(
            Source::Mic,
            "Thursday works for me, I will tell the team.",
            24_000,
            26_500,
        );
        assert!(!is_echo(&reply, &system));
        let later = entry(
            Source::Mic,
            "The release moves to Thursday.",
            40_000,
            42_000,
        );
        assert!(!is_echo(&later, &system));
    }
}
