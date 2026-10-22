//! The words the local model listens for: the user's custom words. Names
//! read in the frontmost window are left out: on the real model, product
//! names among them replaced common words ("lazy dog" heard as "Datadog",
//! "table" as "Tableau"), while the custom words replaced none
//! (`real_engine::boosting_hears_custom_words_and_leaves_other_speech_alone`).

/// FluidAudio rates vocabulary boosting excellent up to 50 terms.
pub const MAX_BOOST_TERMS: usize = 50;

/// Shorter terms cause false replacements, such as "or" heard as "VR".
const MIN_TERM_CHARS: usize = 3;

pub fn boost_terms(custom_words: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    custom_words
        .iter()
        .map(|term| term.trim())
        .filter(|term| term.chars().count() >= MIN_TERM_CHARS)
        .filter(|term| seen.insert(term.to_lowercase()))
        .take(MAX_BOOST_TERMS)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn the_custom_words_keep_their_order() {
        assert_eq!(
            boost_terms(&strings(&["Siobhán", "Kubernetes", "Nguyen"])),
            ["Siobhán", "Kubernetes", "Nguyen"]
        );
    }

    #[test]
    fn a_term_seen_in_any_case_is_kept_once_as_first_written() {
        assert_eq!(
            boost_terms(&strings(&[
                "Siobhán", "SIOBHÁN", "siobhán", "Nguyen", "nguyen"
            ])),
            ["Siobhán", "Nguyen"]
        );
    }

    #[test]
    fn terms_under_three_characters_are_dropped_and_spaces_trimmed() {
        assert_eq!(
            boost_terms(&strings(&["AI", " Zoë ", "", "Bo", "Q3", "Ana"])),
            ["Zoë", "Ana"]
        );
    }

    #[test]
    fn the_list_is_capped_at_the_first_terms() {
        let custom: Vec<String> = (0..80).map(|i| format!("Custom{i}")).collect();
        assert_eq!(boost_terms(&custom), custom[..MAX_BOOST_TERMS]);
    }
}
