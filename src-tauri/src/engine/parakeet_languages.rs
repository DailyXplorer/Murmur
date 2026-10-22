//! The languages Parakeet TDT 0.6B v3 transcribes, as the dictation
//! language setting names them.

const LANGUAGES: [&str; 25] = [
    "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it", "lt", "lv", "mt",
    "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "uk",
];

/// Whether the local model can transcribe dictations in `language`. "auto"
/// is supported: the model detects the language itself.
pub fn supports(language: &str) -> bool {
    language == "auto" || LANGUAGES.contains(&language)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn european_languages_and_auto_are_supported_others_are_not() {
        for language in ["auto", "en", "fr", "de", "es", "uk", "mt"] {
            assert!(supports(language), "{language}");
        }
        for language in ["ja", "zh-Hans", "ko", "ar", "tr", "ca", "no", "", "FR"] {
            assert!(!supports(language), "{language}");
        }
    }
}
