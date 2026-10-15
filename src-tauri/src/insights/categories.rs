use serde::{Deserialize, Serialize};
use specta::Type;

/// Where a dictation was sent, grouped the way the Insights page reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AppCategory {
    AiPrompts,
    PersonalMessages,
    WorkMessages,
    Emails,
    Documents,
    Coding,
    Other,
}

/// Bundle identifiers matched exactly, or by prefix when the entry ends in `*`.
const BUNDLE_RULES: &[(&str, AppCategory)] = &[
    ("com.openai.chat", AppCategory::AiPrompts),
    ("com.openai.codex", AppCategory::AiPrompts),
    ("com.anthropic.claudefordesktop", AppCategory::AiPrompts),
    ("com.todesktop.230313mzl4w4u92", AppCategory::AiPrompts),
    ("com.google.GeminiMacOS", AppCategory::AiPrompts),
    ("ai.perplexity.mac", AppCategory::AiPrompts),
    ("ai.perplexity.comet", AppCategory::AiPrompts),
    ("com.microsoft.copilot*", AppCategory::AiPrompts),
    ("net.whatsapp.WhatsApp", AppCategory::PersonalMessages),
    ("desktop.WhatsApp", AppCategory::PersonalMessages),
    ("com.apple.MobileSMS", AppCategory::PersonalMessages),
    ("ru.keepcoder.Telegram", AppCategory::PersonalMessages),
    ("org.telegram.desktop", AppCategory::PersonalMessages),
    ("org.whispersystems.signal-desktop", AppCategory::PersonalMessages),
    ("com.hnc.Discord", AppCategory::PersonalMessages),
    ("com.facebook.archon*", AppCategory::PersonalMessages),
    ("com.tinyspeck.slackmacgap", AppCategory::WorkMessages),
    ("com.microsoft.teams*", AppCategory::WorkMessages),
    ("com.apple.mail", AppCategory::Emails),
    ("com.microsoft.Outlook", AppCategory::Emails),
    ("com.readdle.smartemail-Mac", AppCategory::Emails),
    ("com.superhuman.electron", AppCategory::Emails),
    ("com.apple.iWork.Pages", AppCategory::Documents),
    ("com.microsoft.Word", AppCategory::Documents),
    ("com.apple.Notes", AppCategory::Documents),
    ("com.apple.TextEdit", AppCategory::Documents),
    ("md.obsidian", AppCategory::Documents),
    ("notion.id", AppCategory::Documents),
    ("com.apple.dt.Xcode", AppCategory::Coding),
    ("com.microsoft.VSCode*", AppCategory::Coding),
    ("dev.zed.Zed*", AppCategory::Coding),
    ("com.jetbrains.*", AppCategory::Coding),
    ("com.sublimetext.*", AppCategory::Coding),
    ("com.apple.Terminal", AppCategory::Coding),
    ("com.googlecode.iterm2", AppCategory::Coding),
    ("dev.warp.Warp*", AppCategory::Coding),
    ("com.mitchellh.ghostty", AppCategory::Coding),
    ("net.kovidgoyal.kitty", AppCategory::Coding),
    ("org.alacritty", AppCategory::Coding),
    ("com.github.wez.wezterm", AppCategory::Coding),
];

/// Hosts matched exactly or as a parent domain (`google.com` does not match
/// `docs.google.com` unless listed; `notion.so` matches `team.notion.so`).
const DOMAIN_RULES: &[(&str, AppCategory)] = &[
    ("chatgpt.com", AppCategory::AiPrompts),
    ("chat.openai.com", AppCategory::AiPrompts),
    ("claude.ai", AppCategory::AiPrompts),
    ("gemini.google.com", AppCategory::AiPrompts),
    ("perplexity.ai", AppCategory::AiPrompts),
    ("copilot.microsoft.com", AppCategory::AiPrompts),
    ("web.whatsapp.com", AppCategory::PersonalMessages),
    ("web.telegram.org", AppCategory::PersonalMessages),
    ("messenger.com", AppCategory::PersonalMessages),
    ("discord.com", AppCategory::PersonalMessages),
    ("app.slack.com", AppCategory::WorkMessages),
    ("teams.microsoft.com", AppCategory::WorkMessages),
    ("teams.live.com", AppCategory::WorkMessages),
    ("mail.google.com", AppCategory::Emails),
    ("gmail.com", AppCategory::Emails),
    ("outlook.com", AppCategory::Emails),
    ("outlook.live.com", AppCategory::Emails),
    ("outlook.office.com", AppCategory::Emails),
    ("outlook.office365.com", AppCategory::Emails),
    ("docs.google.com", AppCategory::Documents),
    ("notion.so", AppCategory::Documents),
    ("notion.site", AppCategory::Documents),
    ("github.com", AppCategory::Coding),
];

/// Browsers whose focused window is queried for the active tab's host.
pub const BROWSER_BUNDLE_IDS: &[&str] = &[
    "com.apple.Safari",
    "com.apple.SafariTechnologyPreview",
    "com.google.Chrome",
    "com.google.Chrome.beta",
    "company.thebrowser.Browser",
    "com.microsoft.edgemac",
    "org.mozilla.firefox",
    "com.brave.Browser",
    "com.operasoftware.Opera",
    "com.vivaldi.Vivaldi",
    "app.zen-browser.zen",
];

pub fn is_browser(bundle_id: &str) -> bool {
    BROWSER_BUNDLE_IDS.contains(&bundle_id)
}

fn bundle_matches(pattern: &str, bundle_id: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => bundle_id.starts_with(prefix),
        None => pattern == bundle_id,
    }
}

fn domain_matches(rule: &str, host: &str) -> bool {
    host == rule
        || host
            .strip_suffix(rule)
            .is_some_and(|subdomain| subdomain.ends_with('.'))
}

/// Maps the app that received a dictation to its category. A known web host
/// wins over the app itself, so Gmail in Safari counts as email.
pub fn categorize(bundle_id: &str, host: Option<&str>) -> AppCategory {
    if let Some(host) = host {
        if let Some((_, category)) = DOMAIN_RULES
            .iter()
            .find(|(rule, _)| domain_matches(rule, host))
        {
            return *category;
        }
    }
    BUNDLE_RULES
        .iter()
        .find(|(pattern, _)| bundle_matches(pattern, bundle_id))
        .map(|(_, category)| *category)
        .unwrap_or(AppCategory::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_native_apps_by_bundle_identifier() {
        let cases = [
            ("com.openai.chat", AppCategory::AiPrompts),
            ("com.todesktop.230313mzl4w4u92", AppCategory::AiPrompts),
            ("net.whatsapp.WhatsApp", AppCategory::PersonalMessages),
            ("com.apple.MobileSMS", AppCategory::PersonalMessages),
            ("com.tinyspeck.slackmacgap", AppCategory::WorkMessages),
            ("com.microsoft.teams2", AppCategory::WorkMessages),
            ("com.apple.mail", AppCategory::Emails),
            ("md.obsidian", AppCategory::Documents),
            ("com.apple.dt.Xcode", AppCategory::Coding),
            ("com.microsoft.VSCodeInsiders", AppCategory::Coding),
            ("com.jetbrains.intellij", AppCategory::Coding),
            ("com.googlecode.iterm2", AppCategory::Coding),
            ("com.example.unknown", AppCategory::Other),
        ];
        for (bundle_id, expected) in cases {
            assert_eq!(categorize(bundle_id, None), expected, "{bundle_id}");
        }
    }

    #[test]
    fn web_host_overrides_the_browser() {
        assert_eq!(
            categorize("com.apple.Safari", Some("mail.google.com")),
            AppCategory::Emails
        );
        assert_eq!(
            categorize("com.google.Chrome", Some("docs.google.com")),
            AppCategory::Documents
        );
        assert_eq!(
            categorize("com.google.Chrome", Some("acme.notion.so")),
            AppCategory::Documents
        );
        assert_eq!(
            categorize("company.thebrowser.Browser", Some("claude.ai")),
            AppCategory::AiPrompts
        );
    }

    #[test]
    fn unknown_or_lookalike_hosts_fall_back_to_the_app() {
        assert_eq!(
            categorize("com.apple.Safari", Some("example.com")),
            AppCategory::Other
        );
        assert_eq!(
            categorize("com.apple.Safari", Some("notclaude.ai")),
            AppCategory::Other
        );
        assert_eq!(
            categorize("com.apple.Safari", Some("google.com")),
            AppCategory::Other
        );
        assert_eq!(categorize("com.apple.Safari", None), AppCategory::Other);
    }

    #[test]
    fn known_browsers_are_queried_for_a_host() {
        assert!(is_browser("com.apple.Safari"));
        assert!(is_browser("com.google.Chrome"));
        assert!(!is_browser("com.apple.mail"));
    }
}
