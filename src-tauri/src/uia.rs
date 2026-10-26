//! UI Automation, the Windows counterpart of `accessibility.rs`. Use it only
//! on a background thread that owns no window, never on the main thread or
//! the WebView's, and never on Murmur's own windows, whose thread may be the
//! one waiting for the answer.

use std::marker::PhantomData;
use std::sync::{Mutex, PoisonError};

use crate::field_excerpt::SelectionText;
use crate::foreground::ForegroundWindow;
use windows::core::BOOL;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationCondition,
    IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextPattern2,
    IUIAutomationTextRange, IUIAutomationValuePattern, TextPatternRangeEndpoint_End,
    TextPatternRangeEndpoint_Start, TextUnit_Character, TreeScope_Children,
    UIA_ControlTypePropertyId, UIA_IsPasswordPropertyId, UIA_NamePropertyId,
    UIA_ProcessIdPropertyId, UIA_TextPattern2Id, UIA_TextPatternId, UIA_ValuePatternId,
    UIA_CONTROLTYPE_ID,
};

/// How long one call waits for an app, as `AX_TIMEOUT_SECONDS` on macOS.
/// Windows otherwise waits 2 s to connect and 20 s for an answer.
const TIMEOUT_MS: u32 = 250;

/// Executables whose windows are never read, as `PASSWORD_APPS` on macOS,
/// and the Windows credential prompt.
const PASSWORD_APPS: [&str; 10] = [
    "1password.exe",
    "bitwarden.exe",
    "keepass.exe",
    "keepassxc.exe",
    "enpass.exe",
    "nordpass.exe",
    "proton pass.exe",
    "dashlane.exe",
    "keeper.exe",
    "credentialuibroker.exe",
];

/// `executable` is a file name such as `foreground::executable_name` gives.
pub fn is_password_app(executable: &str) -> bool {
    PASSWORD_APPS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(executable))
}

/// The foreground window, unless it is Murmur's or a password manager's,
/// or its executable cannot be told.
pub fn frontmost_app() -> Option<ForegroundWindow> {
    let foreground = crate::foreground::window()?;
    if foreground.pid == std::process::id() {
        return None;
    }
    let path = crate::foreground::process_image_path(foreground.pid)?;
    let executable = crate::foreground::executable_name(&path)?;
    (!is_password_app(&executable)).then_some(foreground)
}

/// This thread in COM's multithreaded apartment, left on drop. Not `Send`:
/// it must be dropped on the thread that entered.
struct Apartment(PhantomData<*const ()>);

impl Apartment {
    fn enter() -> windows::core::Result<Self> {
        // SAFETY: plain call. It fails on a thread already in a
        // single-threaded apartment, which is then left as it was.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        Ok(Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful CoInitializeEx in `enter`.
        unsafe { CoUninitialize() };
    }
}

/// A UI Automation client for the calling thread. Each element it returns
/// carries its name, control type, process and whether it is a password
/// field, fetched in the call that found it. Its value is not: `value`
/// asks for it, and never of a password field.
pub struct Automation {
    automation: IUIAutomation2,
    cache: IUIAutomationCacheRequest,
    any: IUIAutomationCondition,
    // Declared last so it drops last: COM stays up while the interfaces
    // above are released.
    _apartment: Apartment,
}

impl Automation {
    pub fn new() -> windows::core::Result<Self> {
        let apartment = Apartment::enter()?;
        // While another thread creates a client, CreateCacheRequest fails
        // with E_FAIL, so creation takes turns.
        static CREATING: Mutex<()> = Mutex::new(());
        let _turn = CREATING.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY: COM is initialised on this thread, and each call returns
        // an owned interface or an error.
        unsafe {
            let automation: IUIAutomation2 =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
            automation.SetConnectionTimeout(TIMEOUT_MS)?;
            automation.SetTransactionTimeout(TIMEOUT_MS)?;
            let cache = automation.CreateCacheRequest()?;
            cache.AddProperty(UIA_ControlTypePropertyId)?;
            cache.AddProperty(UIA_NamePropertyId)?;
            cache.AddProperty(UIA_IsPasswordPropertyId)?;
            cache.AddProperty(UIA_ProcessIdPropertyId)?;
            cache.AddPattern(UIA_ValuePatternId)?;
            let any = automation.CreateTrueCondition()?;
            Ok(Self {
                automation,
                cache,
                any,
                _apartment: apartment,
            })
        }
    }

    /// The keyboard focus, when it is in the process `pid`. A password field
    /// is returned as well; its reader must check.
    pub fn focus_in(&self, pid: u32) -> Option<Element> {
        // SAFETY: returns an owned interface or an error.
        let focused = unsafe { self.automation.GetFocusedElementBuildCache(&self.cache) }.ok()?;
        // SAFETY: reads the cached property.
        let focused_pid = unsafe { focused.CachedProcessId() }.ok()?;
        (u32::try_from(focused_pid).ok() == Some(pid)).then_some(Element(focused))
    }

    /// The window `window`, such as a handle from `foreground::window`.
    pub fn window(&self, window: isize) -> Option<Element> {
        // SAFETY: a stale handle only makes the call fail.
        unsafe {
            self.automation
                .ElementFromHandleBuildCache(HWND(window as *mut _), &self.cache)
        }
        .ok()
        .map(Element)
    }

    /// Whether `a` and `b` are the same element of the app, compared by
    /// runtime id.
    pub fn same(&self, a: &Element, b: &Element) -> bool {
        // SAFETY: both are live elements of this client.
        unsafe { self.automation.CompareElements(&a.0, &b.0) }.is_ok_and(BOOL::as_bool)
    }

    /// The children of `element` in the raw tree. Empty when it has none or
    /// the app did not answer within the timeout.
    pub fn children(&self, element: &Element) -> Vec<Element> {
        // SAFETY: `element` is a live element of this client, and every
        // returned interface is owned.
        unsafe {
            let Ok(children) =
                element
                    .0
                    .FindAllBuildCache(TreeScope_Children, &self.any, &self.cache)
            else {
                return Vec::new();
            };
            let count = children.Length().unwrap_or(0);
            (0..count)
                .filter_map(|index| children.GetElement(index).ok())
                .map(Element)
                .collect()
        }
    }
}

/// An element read with the client's cache request: these getters make no
/// call to the app, except `value`.
pub struct Element(IUIAutomationElement);

impl Element {
    pub fn control_type(&self) -> Option<UIA_CONTROLTYPE_ID> {
        // SAFETY: reads the cached property.
        unsafe { self.0.CachedControlType() }.ok()
    }

    /// The Value pattern's value, when the element has one, in one call to
    /// the app. A password field is never asked, as macOS checks the role
    /// before it reads AXValue.
    pub fn value(&self) -> Option<String> {
        if self.is_secure() {
            return None;
        }
        // SAFETY: reads the cached pattern, then asks the app for its value.
        unsafe {
            self.0
                .GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()?
                .CurrentValue()
                .ok()
        }
        .map(|value| value.to_string())
    }

    pub fn name(&self) -> Option<String> {
        // SAFETY: reads the cached property.
        unsafe { self.0.CachedName() }
            .ok()
            .map(|name| name.to_string())
    }

    /// A password field, or one that cannot tell.
    pub fn is_secure(&self) -> bool {
        // SAFETY: reads the cached property.
        unsafe { self.0.CachedIsPassword() }.map_or(true, BOOL::as_bool)
    }

    /// The whole text of a field, from its Value, else from its text
    /// pattern, which documents give instead. None when it holds more than
    /// `max` UTF-16 units. A password field is never asked.
    pub fn text(&self, max: usize) -> Option<String> {
        let text = match self.value().filter(|value| !value.is_empty()) {
            Some(value) => value,
            None => self.document_text(max)?,
        };
        (text.encode_utf16().count() <= max).then_some(text)
    }

    /// The text pattern's whole document, read up to `max + 1` UTF-16
    /// units, so the app never sends a long document whole. None when it
    /// holds more than `max`, and for a password field.
    pub fn document_text(&self, max: usize) -> Option<String> {
        if self.is_secure() {
            return None;
        }
        let limit = i32::try_from(max).ok()?.checked_add(1)?;
        // SAFETY: each call asks the app and returns an owned interface or
        // an error.
        let text = unsafe {
            self.0
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .ok()?
                .DocumentRange()
                .ok()?
                .GetText(limit)
                .ok()?
        };
        (text.len() <= max).then(|| text.to_string())
    }

    /// The first selection of the element's text, else its caret, which
    /// Chromium gives only through the second text pattern.
    fn text_selection(&self) -> Option<IUIAutomationTextRange> {
        // SAFETY: each call asks the app and returns an owned interface or
        // an error; `active` is a live local.
        unsafe {
            if let Ok(text) = self
                .0
                .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
            {
                return first_selection(&text).or_else(|| {
                    let mut active = BOOL::default();
                    text.GetCaretRange(&mut active).ok()
                });
            }
            let text = self
                .0
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .ok()?;
            first_selection(&text)
        }
    }
}

fn first_selection(text: &IUIAutomationTextPattern) -> Option<IUIAutomationTextRange> {
    // SAFETY: each call returns an owned interface or an error.
    unsafe {
        let selections = text.GetSelection().ok()?;
        if selections.Length().ok()? < 1 {
            return None;
        }
        selections.GetElement(0).ok()
    }
}

/// The text of the focused field next to its selection, as the shared
/// excerpt reader asks for it. Each read is one call to the app.
impl SelectionText for Element {
    type Selection = IUIAutomationTextRange;

    fn is_secure(&self) -> bool {
        Element::is_secure(self)
    }

    fn selection(&self) -> Option<IUIAutomationTextRange> {
        self.text_selection()
    }

    fn before(&self, selection: &IUIAutomationTextRange, max: usize) -> Option<String> {
        let count = i32::try_from(max).ok()?;
        // SAFETY: each call returns an owned interface or an error.
        unsafe {
            let range = selection.Clone().ok()?;
            range
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_End,
                    selection,
                    TextPatternRangeEndpoint_Start,
                )
                .ok()?;
            range
                .MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -count)
                .ok()?;
            // Uncapped: a cap would keep the start, away from the cursor.
            // The range itself holds at most `max` chars.
            range.GetText(-1).ok().map(|text| text.to_string())
        }
    }

    fn after(&self, selection: &IUIAutomationTextRange, max: usize) -> Option<String> {
        let count = i32::try_from(max).ok()?;
        // SAFETY: each call returns an owned interface or an error.
        unsafe {
            let range = selection.Clone().ok()?;
            range
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_Start,
                    selection,
                    TextPatternRangeEndpoint_End,
                )
                .ok()?;
            range
                .MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, count)
                .ok()?;
            range.GetText(count).ok().map(|text| text.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_password_app, Automation};
    use std::sync::{Arc, Barrier};

    #[test]
    fn clients_start_on_several_threads_at_once() {
        let start = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    Automation::new().map(drop)
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().expect("UI Automation client");
        }
    }

    #[test]
    fn password_managers_match_by_executable_in_any_case() {
        for executable in [
            "1password.exe",
            "Bitwarden.exe",
            "KeePassXC.exe",
            "proton pass.exe",
            "CredentialUIBroker.exe",
        ] {
            assert!(is_password_app(executable), "{executable}");
        }
        for executable in ["chrome.exe", "notepad.exe", "keepass", "1password"] {
            assert!(!is_password_app(executable), "{executable}");
        }
    }
}

#[cfg(test)]
pub mod test_fields;

/// Win32 windows for UI Automation tests, each served by a thread of its
/// own as an app serves its windows.
#[cfg(test)]
pub mod test_window {
    use std::sync::{mpsc, Mutex, MutexGuard, Once, PoisonError};
    use std::thread::JoinHandle;
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        PostThreadMessageW, RegisterClassW, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WM_QUIT, WNDCLASSW,
        WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    /// A class that draws nothing, for the top-level window and containers.
    pub const PLAIN: &str = "MurmurUiaTestWindow";
    const PLAIN_CLASS: PCWSTR = w!("MurmurUiaTestWindow");

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    pub fn instance() -> HINSTANCE {
        // SAFETY: the module handle of this executable.
        HINSTANCE(unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap().0)
    }

    /// A visible child window of `parent`; call it on the window's thread.
    pub fn child(parent: HWND, class: &str, text: &str, style: WINDOW_STYLE) -> HWND {
        // SAFETY: `parent` is a live window of this thread.
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                &HSTRING::from(class),
                &HSTRING::from(text),
                WS_CHILD | WS_VISIBLE | style,
                10,
                10,
                440,
                24,
                Some(parent),
                None,
                Some(instance()),
                None,
            )
        }
        .unwrap()
    }

    /// Test windows take turns: reads have budgets of a few hundred
    /// milliseconds, which another test building a large window can use up.
    static OPEN: Mutex<()> = Mutex::new(());

    pub struct TestWindow {
        pub window: isize,
        /// What `build` returned.
        pub handles: Vec<isize>,
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
        _turn: MutexGuard<'static, ()>,
    }

    impl TestWindow {
        /// A top-level window whose children `build` creates on the
        /// window's thread, returning the handles a test needs. Only one
        /// is open at a time.
        pub fn open(build: impl FnOnce(HWND) -> Vec<isize> + Send + 'static) -> Self {
            let turn = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
            let (sender, receiver) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                // SAFETY: registers this test's class once, then creates
                // and serves windows owned by this thread.
                unsafe {
                    static REGISTER: Once = Once::new();
                    REGISTER.call_once(|| {
                        RegisterClassW(&WNDCLASSW {
                            lpfnWndProc: Some(window_proc),
                            hInstance: instance(),
                            lpszClassName: PLAIN_CLASS,
                            ..Default::default()
                        });
                    });
                    let window = CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        PLAIN_CLASS,
                        w!("Murmur UI Automation test"),
                        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                        0,
                        0,
                        480,
                        160,
                        None,
                        None,
                        Some(instance()),
                        None,
                    )
                    .unwrap();
                    let handles = build(window);
                    sender
                        .send((window.0 as isize, handles, GetCurrentThreadId()))
                        .unwrap();
                    let mut message = MSG::default();
                    while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                        DispatchMessageW(&message);
                    }
                    let _ = DestroyWindow(window);
                }
            });
            let (window, handles, thread_id) = receiver.recv().unwrap();
            Self {
                window,
                handles,
                thread_id,
                thread: Some(thread),
                _turn: turn,
            }
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: posts to the window thread's own queue.
            let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
