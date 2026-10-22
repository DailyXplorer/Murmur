//! The words the local model listens for: the user's custom words, then the
//! names read in the frontmost window. The helper lets a name replace a heard
//! word only when their spellings match at 0.75 or more, against 0.50 for a
//! custom word. On the real model with 44 product and people names, FluidAudio's
//! own threshold replaced common words ("lazy dog" heard as "Datadog", "table"
//! as "Tableau", 0.71), 0.70 still did, 0.75 replaced none and fixed "Ghilane"
//! to "Ghislaine" (0.78), and 0.80 fixed nothing
//! (`real_engine::boosting_hears_custom_words_and_leaves_other_speech_alone`).

/// FluidAudio rates vocabulary boosting excellent up to 50 terms.
pub const MAX_BOOST_TERMS: usize = 50;

/// Shorter terms cause false replacements, such as "or" heard as "VR".
const MIN_TERM_CHARS: usize = 3;

/// What one dictation listens for. A term is in one list only.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BoostTerms {
    /// The user's custom words.
    pub words: Vec<String>,
    /// Names read in the frontmost window, held to a closer spelling match.
    pub names: Vec<String>,
}

/// The custom words first, then the window names they leave room for.
pub fn boost_terms(custom_words: &[String], names: &[String]) -> BoostTerms {
    let mut seen = std::collections::HashSet::new();
    let mut usable = |terms: &[String], room: usize| -> Vec<String> {
        terms
            .iter()
            .map(|term| term.trim())
            .filter(|term| term.chars().count() >= MIN_TERM_CHARS)
            .filter(|term| seen.insert(term.to_lowercase()))
            .take(room)
            .map(str::to_string)
            .collect()
    };
    let words = usable(custom_words, MAX_BOOST_TERMS);
    let names = usable(names, MAX_BOOST_TERMS - words.len());
    BoostTerms { words, names }
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
            boost_terms(&strings(&["Siobhán", "Kubernetes", "Nguyen"]), &[]).words,
            ["Siobhán", "Kubernetes", "Nguyen"]
        );
    }

    #[test]
    fn a_term_seen_in_any_case_is_kept_once_as_first_written() {
        assert_eq!(
            boost_terms(
                &strings(&["Siobhán", "SIOBHÁN", "siobhán", "Nguyen", "nguyen"]),
                &strings(&["Ingrid", "INGRID"])
            ),
            BoostTerms {
                words: strings(&["Siobhán", "Nguyen"]),
                names: strings(&["Ingrid"]),
            }
        );
    }

    #[test]
    fn a_name_that_is_also_a_custom_word_is_boosted_as_the_custom_word() {
        assert_eq!(
            boost_terms(
                &strings(&["Siobhán"]),
                &strings(&["Ingrid", "siobhán", "Xiomara"])
            ),
            BoostTerms {
                words: strings(&["Siobhán"]),
                names: strings(&["Ingrid", "Xiomara"]),
            }
        );
    }

    #[test]
    fn terms_under_three_characters_are_dropped_and_spaces_trimmed() {
        assert_eq!(
            boost_terms(
                &strings(&["AI", " Zoë ", "", "Bo", "Q3", "Ana"]),
                &strings(&["UX", " Figma "])
            ),
            BoostTerms {
                words: strings(&["Zoë", "Ana"]),
                names: strings(&["Figma"]),
            }
        );
    }

    #[test]
    fn the_names_fill_only_what_the_custom_words_leave_of_the_cap() {
        let custom: Vec<String> = (0..30).map(|i| format!("Custom{i}")).collect();
        let names: Vec<String> = (0..40).map(|i| format!("Name{i}")).collect();
        let terms = boost_terms(&custom, &names);
        assert_eq!(terms.words, custom);
        assert_eq!(terms.names, names[..MAX_BOOST_TERMS - 30]);
    }

    #[test]
    fn the_custom_words_alone_are_capped_and_leave_no_room_for_names() {
        let custom: Vec<String> = (0..80).map(|i| format!("Custom{i}")).collect();
        let terms = boost_terms(&custom, &strings(&["Ingrid"]));
        assert_eq!(terms.words, custom[..MAX_BOOST_TERMS]);
        assert!(terms.names.is_empty());
    }
}
