//! The dictation style of the kind of app a dictation was started in.

use crate::insights::categories::AppCategory;
use crate::settings::{AppSettings, CategoryStyle, DictationStyle};

/// The style of a dictation into `category`: the one the user set for that
/// kind of app, else the general one. `None` is a kind set to no cleanup.
pub(super) fn style_for(
    settings: &AppSettings,
    category: Option<AppCategory>,
) -> Option<DictationStyle> {
    match category.and_then(|category| settings.category_styles.get(&category)) {
        Some(CategoryStyle::Off) => None,
        Some(CategoryStyle::Formal) => Some(DictationStyle::Formal),
        Some(CategoryStyle::Normal) => Some(DictationStyle::Normal),
        Some(CategoryStyle::Casual) => Some(DictationStyle::Casual),
        None => Some(settings.dictation_style),
    }
}
