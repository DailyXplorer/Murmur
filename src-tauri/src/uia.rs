//! UI Automation, the Windows counterpart of `accessibility.rs`. Use it only
//! on a background thread that owns no window, never on the main thread or
//! the WebView's, and never on Murmur's own windows, whose thread may be the
//! one waiting for the answer.

use std::marker::PhantomData;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation2, IUIAutomationCacheRequest, IUIAutomationCondition,
    IUIAutomationElement, IUIAutomationValuePattern, TreeScope_Children, UIA_ControlTypePropertyId,
    UIA_ValuePatternId, UIA_ValueValuePropertyId, UIA_CONTROLTYPE_ID,
};

/// How long one call waits for an app, as `AX_TIMEOUT_SECONDS` on macOS.
/// Windows otherwise waits 2 s to connect and 20 s for an answer.
const TIMEOUT_MS: u32 = 250;

/// This thread in COM's multithreaded apartment, left on drop. Not `Send`:
/// it must be dropped on the thread that entered.
struct Apartment(PhantomData<*const ()>);

impl Apartment {
    fn enter() -> Option<Self> {
        // SAFETY: plain call. It fails on a thread already in a
        // single-threaded apartment, which is then left as it was.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .ok()?;
        Some(Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful CoInitializeEx in `enter`.
        unsafe { CoUninitialize() };
    }
}

/// A UI Automation client for the calling thread. Each element it returns
/// carries its control type and value, fetched in the call that found it.
pub struct Automation {
    automation: IUIAutomation2,
    cache: IUIAutomationCacheRequest,
    any: IUIAutomationCondition,
    // Declared last so it drops last: COM stays up while the interfaces
    // above are released.
    _apartment: Apartment,
}

impl Automation {
    pub fn new() -> Option<Self> {
        let apartment = Apartment::enter()?;
        // SAFETY: COM is initialised on this thread, and each call returns
        // an owned interface or an error.
        unsafe {
            let automation: IUIAutomation2 =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
            automation.SetConnectionTimeout(TIMEOUT_MS).ok()?;
            automation.SetTransactionTimeout(TIMEOUT_MS).ok()?;
            let cache = automation.CreateCacheRequest().ok()?;
            cache.AddProperty(UIA_ControlTypePropertyId).ok()?;
            cache.AddProperty(UIA_ValueValuePropertyId).ok()?;
            cache.AddPattern(UIA_ValuePatternId).ok()?;
            let any = automation.CreateTrueCondition().ok()?;
            Some(Self {
                automation,
                cache,
                any,
                _apartment: apartment,
            })
        }
    }

    /// The top-level window `window`, a handle from `foreground::window`.
    pub fn window(&self, window: isize) -> Option<Element> {
        // SAFETY: a stale handle only makes the call fail.
        unsafe {
            self.automation
                .ElementFromHandleBuildCache(HWND(window as *mut _), &self.cache)
        }
        .ok()
        .map(Element)
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
/// call to the app.
pub struct Element(IUIAutomationElement);

impl Element {
    pub fn control_type(&self) -> Option<UIA_CONTROLTYPE_ID> {
        // SAFETY: reads the cached property.
        unsafe { self.0.CachedControlType() }.ok()
    }

    /// The Value pattern's value, when the element has one.
    pub fn value(&self) -> Option<String> {
        // SAFETY: reads the cached pattern and its cached property.
        unsafe {
            self.0
                .GetCachedPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .ok()?
                .CachedValue()
                .ok()
        }
        .map(|value| value.to_string())
    }
}
