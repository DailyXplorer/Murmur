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
    let host = super::categories::is_browser(&bundle_id)
        .then(crate::uia::Automation::new)
        .flatten()
        .and_then(|automation| browser::tab_host(&automation, foreground.window));
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
        use crate::uia::Automation;
        use std::sync::{mpsc, Once};
        use std::thread::JoinHandle;
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
            PostThreadMessageW, RegisterClassW, MSG, WINDOW_EX_STYLE, WM_QUIT, WNDCLASSW,
            WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };

        const CLASS_NAME: PCWSTR = w!("MurmurUiaTestWindow");

        unsafe extern "system" fn window_proc(
            window: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }

        /// A top-level window holding one EDIT control, served by a thread
        /// of its own, as a browser serves its address bar.
        struct EditWindow {
            window: isize,
            thread_id: u32,
            thread: Option<JoinHandle<()>>,
        }

        impl EditWindow {
            fn open(text: &str) -> Self {
                let text = HSTRING::from(text);
                let (sender, receiver) = mpsc::channel();
                let thread = std::thread::spawn(move || {
                    // SAFETY: registers this test's class once, then creates
                    // and serves windows owned by this thread.
                    unsafe {
                        let instance = HINSTANCE(GetModuleHandleW(PCWSTR::null()).unwrap().0);
                        static REGISTER: Once = Once::new();
                        REGISTER.call_once(|| {
                            RegisterClassW(&WNDCLASSW {
                                lpfnWndProc: Some(window_proc),
                                hInstance: instance,
                                lpszClassName: CLASS_NAME,
                                ..Default::default()
                            });
                        });
                        let window = CreateWindowExW(
                            WINDOW_EX_STYLE::default(),
                            CLASS_NAME,
                            w!("Murmur UI Automation test"),
                            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                            0,
                            0,
                            480,
                            160,
                            None,
                            None,
                            Some(instance),
                            None,
                        )
                        .unwrap();
                        CreateWindowExW(
                            WINDOW_EX_STYLE::default(),
                            w!("EDIT"),
                            &text,
                            WS_CHILD | WS_VISIBLE | WS_BORDER,
                            10,
                            10,
                            440,
                            24,
                            Some(window),
                            None,
                            Some(instance),
                            None,
                        )
                        .unwrap();
                        sender
                            .send((window.0 as isize, GetCurrentThreadId()))
                            .unwrap();
                        let mut message = MSG::default();
                        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                            DispatchMessageW(&message);
                        }
                        let _ = DestroyWindow(window);
                    }
                });
                let (window, thread_id) = receiver.recv().unwrap();
                Self {
                    window,
                    thread_id,
                    thread: Some(thread),
                }
            }
        }

        impl Drop for EditWindow {
            fn drop(&mut self) {
                // SAFETY: posts to the window thread's own queue.
                let _ =
                    unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
            }
        }

        #[test]
        fn reads_the_host_from_an_address_bar_edit() {
            let window = EditWindow::open("github.com/DailyXplorer/Murmur");
            let automation = Automation::new().expect("UI Automation client");
            assert_eq!(
                tab_host(&automation, window.window).as_deref(),
                Some("github.com")
            );
        }

        #[test]
        fn a_search_in_the_address_bar_gives_no_host() {
            let window = EditWindow::open("best pizza near me");
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
