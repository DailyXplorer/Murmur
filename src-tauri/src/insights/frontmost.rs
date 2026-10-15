/// The app that received a dictation. `host` is only the domain of the
/// active browser tab, never the full URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictationTarget {
    pub bundle_id: String,
    pub host: Option<String>,
}

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

pub fn current() -> Option<DictationTarget> {
    let application = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = application.processIdentifier();
    if pid <= 0 || u32::try_from(pid).ok() == Some(std::process::id()) {
        return None;
    }
    let bundle_id = application.bundleIdentifier()?.to_string();
    if bundle_id.is_empty() {
        return None;
    }
    let host = super::categories::is_browser(&bundle_id)
        .then(|| macos::focused_window_url(pid))
        .flatten()
        .and_then(|url| host_of(&url));
    Some(DictationTarget { bundle_id, host })
}

mod macos {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation::url::CFURL;
    use std::ptr;

    type AXUIElementRef = CFTypeRef;
    const AX_SUCCESS: i32 = 0;
    /// A hung browser must not stall the insights thread for the system
    /// default of several seconds.
    const AX_TIMEOUT_SECONDS: f32 = 0.5;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
    }

    fn attribute(element: &CFType, name: &str) -> Option<CFType> {
        let name = CFString::new(name);
        let mut value: CFTypeRef = ptr::null();
        // SAFETY: `element` is a live AXUIElement and `value` receives a +1
        // reference that `wrap_under_create_rule` takes ownership of.
        let status = unsafe {
            AXUIElementCopyAttributeValue(
                element.as_CFTypeRef(),
                name.as_concrete_TypeRef(),
                &mut value,
            )
        };
        (status == AX_SUCCESS && !value.is_null())
            .then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn as_url_string(value: &CFType) -> Option<String> {
        if let Some(string) = value.downcast::<CFString>() {
            return Some(string.to_string());
        }
        value
            .downcast::<CFURL>()
            .map(|url| url.get_string().to_string())
    }

    /// Reads `AXDocument`, then `AXURL`, from the app's focused window.
    pub fn focused_window_url(pid: i32) -> Option<String> {
        // SAFETY: the create call returns a +1 AXUIElement or null.
        let app = unsafe {
            let raw = AXUIElementCreateApplication(pid);
            if raw.is_null() {
                return None;
            }
            AXUIElementSetMessagingTimeout(raw, AX_TIMEOUT_SECONDS);
            CFType::wrap_under_create_rule(raw)
        };
        let window = attribute(&app, "AXFocusedWindow")?;
        ["AXDocument", "AXURL"]
            .iter()
            .find_map(|name| attribute(&window, name).and_then(|v| as_url_string(&v)))
    }
}

#[cfg(test)]
mod tests {
    use super::host_of;

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
}
