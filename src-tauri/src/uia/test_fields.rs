//! Two fields served by a UI Automation provider of the test's own, as an
//! app's own UI serves them: a password field and a plain one. Each gives
//! its text by Value and by text pattern, and counts how often it is asked.

use super::test_window::{child, instance, TestWindow};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;
use windows::core::{implement, Error, IUnknown, Ref, Result, BOOL, BSTR, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    E_NOTIMPL, HWND, LPARAM, LRESULT, VARIANT_FALSE, VARIANT_TRUE, WPARAM,
};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Variant::{VARIANT, VT_BOOL};
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, IRawElementProviderSimple_Impl, ITextProvider, ITextProvider_Impl,
    ITextRangeProvider, ITextRangeProvider_Impl, IValueProvider, IValueProvider_Impl,
    ProviderOptions, ProviderOptions_ServerSideProvider, SupportedTextSelection,
    SupportedTextSelection_None, TextPatternRangeEndpoint, TextUnit, UIA_IsPasswordPropertyId,
    UIA_TextPattern2Id, UIA_TextPatternId, UIA_ValuePatternId, UIA_ValueValuePropertyId,
    UiaHostProviderFromHwnd, UiaPoint, UiaReturnRawElementProvider, UiaRootObjectId,
    UIA_PATTERN_ID, UIA_PROPERTY_ID, UIA_TEXTATTRIBUTE_ID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, RegisterClassW, WINDOW_STYLE, WM_GETOBJECT, WNDCLASSW, WNDPROC,
};

pub const SECRET_TEXT: &str = "hunter2 Corvina";
pub const PLAIN_TEXT: &str = "Dear Ingrid";

const SECRET_FIELD: &str = "MurmurUiaTestSecretField";
const PLAIN_FIELD: &str = "MurmurUiaTestPlainField";

/// How a field is asked for its text.
#[derive(Clone, Copy)]
pub enum Read {
    /// The Value property or the Value pattern's value.
    Value,
    /// The text pattern, either version, or a text range's text.
    Text,
}

/// Requests of the plain field, then of the password field, by `Read`.
static REQUESTS: [[AtomicUsize; 2]; 2] = [
    [AtomicUsize::new(0), AtomicUsize::new(0)],
    [AtomicUsize::new(0), AtomicUsize::new(0)],
];

fn count(secure: bool, read: Read) {
    REQUESTS[usize::from(secure)][read as usize].fetch_add(1, Ordering::SeqCst);
}

/// How often the password field (`secure`) or the plain one was asked for
/// its text by `read` since `open`.
pub fn requests(secure: bool, read: Read) -> usize {
    REQUESTS[usize::from(secure)][read as usize].load(Ordering::SeqCst)
}

/// A window holding the password field, then the plain one, in `handles`.
/// Test windows take turns, so the counts are this window's alone.
pub fn open() -> TestWindow {
    TestWindow::open(|window| {
        register();
        for request in REQUESTS.iter().flatten() {
            request.store(0, Ordering::SeqCst);
        }
        vec![
            child(window, SECRET_FIELD, "Password", WINDOW_STYLE::default()).0 as isize,
            child(window, PLAIN_FIELD, "Message", WINDOW_STYLE::default()).0 as isize,
        ]
    })
}

fn text(secure: bool) -> &'static str {
    if secure {
        SECRET_TEXT
    } else {
        PLAIN_TEXT
    }
}

fn not_implemented<T>() -> Result<T> {
    Err(E_NOTIMPL.into())
}

#[implement(IRawElementProviderSimple)]
struct FieldProvider {
    window: isize,
    secure: bool,
}

impl IRawElementProviderSimple_Impl for FieldProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let secure = self.secure;
        if pattern == UIA_ValuePatternId {
            // Not counted: the client's cache request asks every element
            // for this pattern, without its value.
            let value: IValueProvider = FieldValue { secure }.into();
            return Ok(value.into());
        }
        if pattern == UIA_TextPatternId || pattern == UIA_TextPattern2Id {
            count(secure, Read::Text);
        }
        if pattern != UIA_TextPatternId {
            // S_OK and no pattern.
            return Err(Error::empty());
        }
        let text: ITextProvider = FieldText { secure }.into();
        Ok(text.into())
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let mut variant = VARIANT::default();
        if property == UIA_ValueValuePropertyId {
            count(self.secure, Read::Value);
        } else if property == UIA_IsPasswordPropertyId {
            // SAFETY: writes a VT_BOOL variant's tag and value.
            unsafe {
                let inner = &mut *variant.Anonymous.Anonymous;
                inner.vt = VT_BOOL;
                inner.Anonymous.boolVal = if self.secure {
                    VARIANT_TRUE
                } else {
                    VARIANT_FALSE
                };
            }
        }
        // Empty: UI Automation asks the Value pattern or the window.
        Ok(variant)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        // SAFETY: the provider's own window.
        unsafe { UiaHostProviderFromHwnd(HWND(self.window as *mut _)) }
    }
}

#[implement(IValueProvider)]
struct FieldValue {
    secure: bool,
}

impl IValueProvider_Impl for FieldValue_Impl {
    fn SetValue(&self, _: &PCWSTR) -> Result<()> {
        not_implemented()
    }

    fn Value(&self) -> Result<BSTR> {
        count(self.secure, Read::Value);
        Ok(BSTR::from(text(self.secure)))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(true.into())
    }
}

/// The text pattern, whose document is the field's text. Only what a
/// whole-document read needs is implemented.
#[implement(ITextProvider)]
struct FieldText {
    secure: bool,
}

impl ITextProvider_Impl for FieldText_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        not_implemented()
    }

    fn GetVisibleRanges(&self) -> Result<*mut SAFEARRAY> {
        not_implemented()
    }

    fn RangeFromChild(&self, _: Ref<'_, IRawElementProviderSimple>) -> Result<ITextRangeProvider> {
        not_implemented()
    }

    fn RangeFromPoint(&self, _: &UiaPoint) -> Result<ITextRangeProvider> {
        not_implemented()
    }

    fn DocumentRange(&self) -> Result<ITextRangeProvider> {
        Ok(FieldRange {
            secure: self.secure,
        }
        .into())
    }

    fn SupportedTextSelection(&self) -> Result<SupportedTextSelection> {
        Ok(SupportedTextSelection_None)
    }
}

/// The range of the whole document.
#[implement(ITextRangeProvider)]
struct FieldRange {
    secure: bool,
}

impl ITextRangeProvider_Impl for FieldRange_Impl {
    fn Clone(&self) -> Result<ITextRangeProvider> {
        Ok(FieldRange {
            secure: self.secure,
        }
        .into())
    }

    fn Compare(&self, _: Ref<'_, ITextRangeProvider>) -> Result<BOOL> {
        not_implemented()
    }

    fn CompareEndpoints(
        &self,
        _: TextPatternRangeEndpoint,
        _: Ref<'_, ITextRangeProvider>,
        _: TextPatternRangeEndpoint,
    ) -> Result<i32> {
        not_implemented()
    }

    fn ExpandToEnclosingUnit(&self, _: TextUnit) -> Result<()> {
        not_implemented()
    }

    fn FindAttribute(
        &self,
        _: UIA_TEXTATTRIBUTE_ID,
        _: &VARIANT,
        _: BOOL,
    ) -> Result<ITextRangeProvider> {
        not_implemented()
    }

    fn FindText(&self, _: &BSTR, _: BOOL, _: BOOL) -> Result<ITextRangeProvider> {
        not_implemented()
    }

    fn GetAttributeValue(&self, _: UIA_TEXTATTRIBUTE_ID) -> Result<VARIANT> {
        not_implemented()
    }

    fn GetBoundingRectangles(&self) -> Result<*mut SAFEARRAY> {
        not_implemented()
    }

    fn GetEnclosingElement(&self) -> Result<IRawElementProviderSimple> {
        not_implemented()
    }

    /// The text, cut to `max` units unless it is -1. It is ASCII, so a
    /// char is a UTF-16 unit.
    fn GetText(&self, max: i32) -> Result<BSTR> {
        count(self.secure, Read::Text);
        let text = text(self.secure);
        let len = usize::try_from(max).map_or(text.len(), |max| max.min(text.len()));
        Ok(BSTR::from(&text[..len]))
    }

    fn Move(&self, _: TextUnit, _: i32) -> Result<i32> {
        not_implemented()
    }

    fn MoveEndpointByUnit(&self, _: TextPatternRangeEndpoint, _: TextUnit, _: i32) -> Result<i32> {
        not_implemented()
    }

    fn MoveEndpointByRange(
        &self,
        _: TextPatternRangeEndpoint,
        _: Ref<'_, ITextRangeProvider>,
        _: TextPatternRangeEndpoint,
    ) -> Result<()> {
        not_implemented()
    }

    fn Select(&self) -> Result<()> {
        not_implemented()
    }

    fn AddToSelection(&self) -> Result<()> {
        not_implemented()
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        not_implemented()
    }

    fn ScrollIntoView(&self, _: BOOL) -> Result<()> {
        not_implemented()
    }

    fn GetChildren(&self) -> Result<*mut SAFEARRAY> {
        not_implemented()
    }
}

unsafe extern "system" fn field_proc<const SECURE: bool>(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_GETOBJECT && lparam.0 as i32 == UiaRootObjectId {
        let provider: IRawElementProviderSimple = FieldProvider {
            window: window.0 as isize,
            secure: SECURE,
        }
        .into();
        // SAFETY: answers this window's own WM_GETOBJECT.
        return unsafe { UiaReturnRawElementProvider(window, wparam, lparam, &provider) };
    }
    // SAFETY: the default handling of this window's message.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn register() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        let classes: [(&str, WNDPROC); 2] = [
            (SECRET_FIELD, Some(field_proc::<true>)),
            (PLAIN_FIELD, Some(field_proc::<false>)),
        ];
        for (class, window_proc) in classes {
            let class = HSTRING::from(class);
            // SAFETY: registers a class whose procedure lives as long as
            // the test executable; the name is copied.
            unsafe {
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: window_proc,
                    hInstance: instance(),
                    lpszClassName: PCWSTR(class.as_ptr()),
                    ..Default::default()
                })
            };
        }
    });
}
