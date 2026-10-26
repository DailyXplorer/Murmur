/// The app that received a dictation. `host` is only the domain of the
/// active browser tab, never the full URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictationTarget {
    pub bundle_id: String,
    pub host: Option<String>,
}

/// The app in front, named by its executable. For a browser, the host of
/// its active tab is read through UI Automation, so call this off the main
/// thread.
#[cfg(target_os = "windows")]
pub fn current() -> Option<DictationTarget> {
    let foreground = crate::foreground::window()?;
    if foreground.pid == std::process::id() {
        return None;
    }
    let path = crate::foreground::process_image_path(foreground.pid)?;
    let bundle_id = crate::foreground::executable_name(&path)?;
    let host = if super::categories::is_browser(&bundle_id) {
        match crate::uia::Automation::new() {
            Ok(automation) => browser::tab_host(&automation, foreground.window),
            Err(e) => {
                log::debug!("UI Automation is unavailable for the browser's tab: {e}");
                None
            }
        }
    } else {
        None
    };
    Some(DictationTarget { bundle_id, host })
}

/// The active tab of a Windows browser, as UI Automation shows it.
#[cfg(target_os = "windows")]
mod browser {
    use super::super::host::{self, TabText};
    use crate::uia::{Automation, Element};
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};
    use windows::Win32::UI::Accessibility::{UIA_DocumentControlTypeId, UIA_EditControlTypeId};

    /// A browser's own interface is a few hundred elements. The page, which
    /// can hold thousands, is never entered.
    const MAX_ELEMENTS: usize = 500;
    /// What one lookup may spend, so a hung browser stalls the insights
    /// thread for about one UI Automation timeout.
    const BUDGET: Duration = Duration::from_millis(250);

    /// The host of the tab shown in the browser window `window`: the page's
    /// URL, else the address bar's text.
    pub fn tab_host(automation: &Automation, window: isize) -> Option<String> {
        host::tab_host(tab_texts(automation, window))
    }

    /// Walks the window breadth first and yields the value of each page (a
    /// document) and of each edit box met outside the pages. Neither is
    /// entered, so nothing typed in a page is read.
    fn tab_texts(automation: &Automation, window: isize) -> impl Iterator<Item = TabText> + '_ {
        let deadline = Instant::now() + BUDGET;
        let mut queue: VecDeque<Element> = automation.window(window).into_iter().collect();
        let mut visited = 0;
        std::iter::from_fn(move || loop {
            if visited == MAX_ELEMENTS || Instant::now() >= deadline {
                return None;
            }
            let element = queue.pop_front()?;
            visited += 1;
            match element.control_type() {
                Some(kind) if kind == UIA_DocumentControlTypeId => {
                    if let Some(url) = element.value() {
                        return Some(TabText::Page(url));
                    }
                }
                Some(kind) if kind == UIA_EditControlTypeId => {
                    if let Some(text) = element.value() {
                        return Some(TabText::AddressBar(text));
                    }
                }
                _ => queue.extend(automation.children(&element)),
            }
        })
    }

    #[cfg(test)]
    mod tests {
        use super::tab_host;
        use crate::uia::test_window::{child, TestWindow};
        use crate::uia::Automation;
        use windows::Win32::UI::WindowsAndMessaging::WS_BORDER;

        /// A window holding one EDIT control, as a browser holds its
        /// address bar.
        fn edit_window(text: &'static str) -> TestWindow {
            TestWindow::open(move |window| {
                child(window, "EDIT", text, WS_BORDER);
                Vec::new()
            })
        }

        #[test]
        fn reads_the_host_from_an_address_bar_edit() {
            let window = edit_window("github.com/DailyXplorer/Murmur");
            let automation = Automation::new().expect("UI Automation client");
            assert_eq!(
                tab_host(&automation, window.window).as_deref(),
                Some("github.com")
            );
        }

        #[test]
        fn a_search_in_the_address_bar_gives_no_host() {
            let window = edit_window("best pizza near me");
            let automation = Automation::new().expect("UI Automation client");
            assert_eq!(tab_host(&automation, window.window), None);
        }
    }
}

#[cfg(target_os = "macos")]
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
        .and_then(|url| super::host::host_of(&url));
    Some(DictationTarget { bundle_id, host })
}

#[cfg(target_os = "macos")]
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
