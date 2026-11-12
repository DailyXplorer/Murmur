//! The dictation style of the kind of app a dictation goes into.

use crate::insights::categories::{categorize, AppCategory};
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

/// The kind of app in front, read only when a kind has a style of its own.
/// Off the async thread: a browser's host comes from UI Automation on Windows.
pub(super) async fn dictation_category(settings: &AppSettings) -> Option<AppCategory> {
    if settings.category_styles.is_empty() {
        return None;
    }
    tauri::async_runtime::spawn_blocking(|| {
        crate::insights::frontmost::current()
            .map(|target| categorize(&target.bundle_id, target.host.as_deref()))
    })
    .await
    .ok()
    .flatten()
}
