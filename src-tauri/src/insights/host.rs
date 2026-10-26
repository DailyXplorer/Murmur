//! The domain of a browser's active tab, from what the browser shows. Only
//! the host is ever kept, never the full URL.

/// Reduces a URL to its lowercase host without a leading `www.`. Returns
/// `None` for non-web documents such as `file://` or `about:blank`.
pub fn host_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = host_port.split(':').next()?.trim_end_matches('.');
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

/// Text a browser window shows, in the order a walk of its interface met it.
#[cfg(any(target_os = "windows", test))]
pub enum TabText {
    /// The value of a page (a UI Automation document): its URL.
    Page(String),
    /// The value of an edit box outside every page: the address bar.
    AddressBar(String),
}

/// The active tab's host: the first page with a web URL, else the first
/// address bar. Stops reading at the first page that names a host.
#[cfg(any(target_os = "windows", test))]
pub fn tab_host(texts: impl IntoIterator<Item = TabText>) -> Option<String> {
    let mut address_bar = None;
    for text in texts {
        match text {
            TabText::Page(url) => {
                if let Some(host) = host_of(&url) {
                    return Some(host);
                }
            }
            TabText::AddressBar(text) => {
                address_bar.get_or_insert(text);
            }
        }
    }
    address_bar.and_then(|text| address_bar_host(&text))
}

/// The host in what an address bar shows. Chrome and Edge hide the
/// `https://`, and the bar may hold a search the user is still typing, so
/// only text shaped like a domain gives a host.
#[cfg(any(target_os = "windows", test))]
pub fn address_bar_host(text: &str) -> Option<String> {
    let text = text.trim();
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    let host = if text.contains("://") {
        host_of(text)?
    } else {
        host_of(&format!("https://{text}"))?
    };
    let is_domain = host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        });
    is_domain.then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_host_only() {
        assert_eq!(
            host_of("https://mail.google.com/mail/u/0/#inbox").as_deref(),
            Some("mail.google.com")
        );
        assert_eq!(
            host_of("HTTPS://WWW.Notion.so/page?x=1").as_deref(),
            Some("notion.so")
        );
        assert_eq!(
            host_of("http://user:pw@claude.ai:8443/chat").as_deref(),
            Some("claude.ai")
        );
        assert_eq!(
            host_of("https://chatgpt.com").as_deref(),
            Some("chatgpt.com")
        );
    }

    #[test]
    fn rejects_non_web_documents() {
        assert_eq!(host_of("file:///tmp/notes.txt"), None);
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("https:///path"), None);
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn an_address_bar_without_a_scheme_gives_its_host() {
        assert_eq!(
            address_bar_host("github.com/DailyXplorer/Murmur").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            address_bar_host("  mail.google.com/mail/u/0/#inbox ").as_deref(),
            Some("mail.google.com")
        );
        assert_eq!(
            address_bar_host("https://www.notion.so/page").as_deref(),
            Some("notion.so")
        );
        assert_eq!(
            address_bar_host("http://localhost.test:8080").as_deref(),
            Some("localhost.test")
        );
    }

    #[test]
    fn an_address_bar_holding_a_search_gives_no_host() {
        assert_eq!(address_bar_host("best pizza near me"), None);
        assert_eq!(address_bar_host("murmur"), None);
        assert_eq!(address_bar_host("what is 2+2?"), None);
        assert_eq!(address_bar_host("chrome://settings"), None);
        assert_eq!(address_bar_host("about:blank"), None);
        assert_eq!(address_bar_host("a..b"), None);
        assert_eq!(address_bar_host(""), None);
    }

    #[test]
    fn the_page_url_wins_over_the_address_bar() {
        let texts = [
            TabText::AddressBar("github.com/DailyXplorer".into()),
            TabText::Page("https://mail.google.com/mail/u/0/".into()),
        ];
        assert_eq!(tab_host(texts).as_deref(), Some("mail.google.com"));
    }

    #[test]
    fn a_page_without_a_web_url_falls_back_to_the_address_bar() {
        let texts = [
            TabText::Page("chrome://newtab/".into()),
            TabText::AddressBar("github.com/DailyXplorer".into()),
            TabText::AddressBar("second.example.com".into()),
        ];
        assert_eq!(tab_host(texts).as_deref(), Some("github.com"));
        assert_eq!(tab_host([]), None);
        assert_eq!(tab_host([TabText::AddressBar("a search".into())]), None);
    }

    #[test]
    fn reading_stops_at_the_first_page_with_a_host() {
        let mut read = 0;
        let texts = ["https://claude.ai/new", "https://github.com"]
            .into_iter()
            .inspect(|_| read += 1)
            .map(|url| TabText::Page(url.into()));
        assert_eq!(tab_host(texts).as_deref(), Some("claude.ai"));
        assert_eq!(read, 1);
    }
}
