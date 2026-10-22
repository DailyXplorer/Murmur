//! Tray menu internationalization
//!
//! Everything is auto-generated at compile time by build.rs from the
//! frontend locale files (src/i18n/locales/*/translation.json).
//!
//! The English translation.json is the single source of truth:
//! - TrayStrings struct fields are derived from the English "tray" keys
//! - All languages are auto-discovered from the locales directory
//!
//! To add a new tray menu item:
//! 1. Add the key to en/translation.json under "tray"
//! 2. Add translations to other locale files
//! 3. Update tray.rs to use the new field (e.g., strings.new_field)

use once_cell::sync::Lazy;
use std::collections::HashMap;

include!(concat!(env!("OUT_DIR"), "/tray_translations.rs"));

/// Get localized tray menu strings for `locale`, the app language setting.
///
/// Lookup order: exact locale → language code → English.
pub fn get_tray_translations(locale: Option<String>) -> TrayStrings {
    let normalized = locale
        .as_deref()
        .unwrap_or("en")
        .to_lowercase()
        .replace('_', "-");
    let language = normalized.split('-').next().unwrap_or("en");

    let exact_match = TRANSLATIONS
        .iter()
        .find_map(|(code, strings)| code.eq_ignore_ascii_case(&normalized).then_some(strings));

    exact_match
        .or_else(|| TRANSLATIONS.get(language))
        .or_else(|| TRANSLATIONS.get("en"))
        .cloned()
        .expect("English translations must exist")
}

#[cfg(test)]
mod tests {
    use super::{get_tray_translations, TRANSLATIONS};

    #[test]
    fn resolves_locale_fallbacks() {
        for (locale, expected) in [
            ("fr-FR", "fr"),
            ("pt-BR", "pt"),
            ("pt_PT", "pt"),
            ("IT", "it"),
            ("ja", "en"),
            ("zh-TW", "en"),
            ("zh-Hant-HK", "en"),
            ("ar", "en"),
            ("xx-YY", "en"),
        ] {
            assert_eq!(
                format!("{:?}", get_tray_translations(Some(locale.into()))),
                format!("{:?}", TRANSLATIONS[expected]),
                "{locale} should resolve to {expected}"
            );
        }
    }
}
