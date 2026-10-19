//! Chinese in the script the user chose. Codex is asked for "zh" only and
//! writes Simplified or Traditional characters as it likes, so its text is
//! converted with OpenCC afterwards.

use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use std::sync::OnceLock;

/// `text` in the script of `language` when it is "zh-Hans" or "zh-Hant".
/// Any other language leaves it as it is, so automatic detection never
/// rewrites the characters of Japanese or other CJK text. So does an OpenCC
/// that cannot start.
pub fn in_chosen_script(language: Option<&str>, text: &str) -> String {
    match language.and_then(converter) {
        Some(converter) => converter.convert(text),
        None => text.to_string(),
    }
}

/// The converter to the script of `language`, built on first use: building
/// one parses its dictionaries, too slow to repeat for every meeting piece.
fn converter(language: &str) -> Option<&'static OpenCC> {
    static SIMPLIFIED: OnceLock<Option<OpenCC>> = OnceLock::new();
    static TRADITIONAL: OnceLock<Option<OpenCC>> = OnceLock::new();
    let (cell, config) = match language {
        "zh-Hans" => (&SIMPLIFIED, BuiltinConfig::Tw2sp),
        "zh-Hant" => (&TRADITIONAL, BuiltinConfig::S2tw),
        _ => return None,
    };
    cell.get_or_init(|| match OpenCC::from_config(config) {
        Ok(converter) => Some(converter),
        Err(error) => {
            log::error!("Cannot start OpenCC, Chinese keeps the script Codex wrote: {error}");
            None
        }
    })
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_is_written_in_the_chosen_script() {
        assert_eq!(
            in_chosen_script(Some("zh-Hant"), "我们开会吧。"),
            "我們開會吧。"
        );
        assert_eq!(
            in_chosen_script(Some("zh-Hans"), "我們開會吧。"),
            "我们开会吧。"
        );
    }

    #[test]
    fn other_languages_keep_their_characters() {
        for language in [None, Some("auto"), Some("ja")] {
            assert_eq!(in_chosen_script(language, "会議と會議"), "会議と會議");
        }
    }
}
