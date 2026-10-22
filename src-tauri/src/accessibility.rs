//! Read-only Accessibility queries of other apps: the focused text field and
//! the text of the frontmost window. Needs the Accessibility permission
//! Murmur already holds for pasting; no screenshots, no Screen Recording.
//! Secure text fields and password managers are never read.

use crate::field_excerpt::Utf16Range;
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeID, CFTypeRef, TCFType};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;

type AXUIElementRef = CFTypeRef;
const AX_SUCCESS: i32 = 0;
/// kAXValueCFRangeType.
const AX_VALUE_CF_RANGE: u32 = 4;

#[repr(C)]
#[derive(Default)]
struct CFRange {
    location: isize,
    length: isize,
}
/// A hung app must not stall a reader for the system default of seconds.
const AX_TIMEOUT_SECONDS: f32 = 0.25;
const SECURE_TEXT_FIELD: &str = "AXSecureTextField";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

/// Bundle identifiers whose windows are never read.
const PASSWORD_APPS: [&str; 12] = [
    "com.apple.keychainaccess",
    "com.apple.Passwords",
    "com.1password.1password",
    "com.agilebits.onepassword7",
    "com.agilebits.onepassword-osx",
    "com.bitwarden.desktop",
    "org.keepassxc.keepassxc",
    "com.dashlane.dashlanephonefinal",
    "com.lastpass.LastPass",
    "in.sinew.Enpass-Desktop",
    "com.nordpass.macos.NordPass",
    "me.proton.pass.electron",
];

pub fn is_password_app(bundle_id: &str) -> bool {
    PASSWORD_APPS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(bundle_id))
}

/// A retained accessibility element.
#[derive(Clone, PartialEq)]
pub struct Element(CFType);

impl Element {
    fn from_create(raw: AXUIElementRef) -> Option<Self> {
        if raw.is_null() {
            return None;
        }
        Self::from_value(unsafe { CFType::wrap_under_create_rule(raw) })
    }

    /// The timeout is per element, so every element gets it.
    fn from_value(value: CFType) -> Option<Self> {
        if value.type_of() != unsafe { AXUIElementGetTypeID() } {
            return None;
        }
        unsafe { AXUIElementSetMessagingTimeout(value.as_CFTypeRef(), AX_TIMEOUT_SECONDS) };
        Some(Self(value))
    }

    fn raw(&self) -> AXUIElementRef {
        self.0.as_CFTypeRef()
    }

    pub fn application(pid: i32) -> Option<Self> {
        Self::from_create(unsafe { AXUIElementCreateApplication(pid) })
    }

    pub fn system_wide() -> Option<Self> {
        Self::from_create(unsafe { AXUIElementCreateSystemWide() })
    }

    fn has(&self, name: &str) -> bool {
        self.attribute(name).is_some()
    }

    fn attribute(&self, name: &str) -> Option<CFType> {
        let name = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(self.raw(), name.as_concrete_TypeRef(), &mut value)
        };
        (status == AX_SUCCESS && !value.is_null())
            .then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    pub fn element(&self, name: &str) -> Option<Element> {
        self.attribute(name).and_then(Self::from_value)
    }

    pub fn string(&self, name: &str) -> Option<String> {
        self.attribute(name)?
            .downcast::<CFString>()
            .map(|value| value.to_string())
    }

    pub fn number(&self, name: &str) -> Option<i64> {
        self.attribute(name)?.downcast::<CFNumber>()?.to_i64()
    }

    /// A range attribute such as AXSelectedTextRange.
    pub fn range(&self, name: &str) -> Option<Utf16Range> {
        let value = self.attribute(name)?;
        if value.type_of() != unsafe { AXValueGetTypeID() } {
            return None;
        }
        let mut range = CFRange::default();
        let read = unsafe {
            AXValueGetValue(
                value.as_CFTypeRef(),
                AX_VALUE_CF_RANGE,
                (&mut range as *mut CFRange).cast(),
            )
        };
        if !read {
            return None;
        }
        Some(Utf16Range {
            start: usize::try_from(range.location).ok()?,
            len: usize::try_from(range.length).ok()?,
        })
    }

    /// AXStringForRange: part of a text element's value.
    pub fn string_for_range(&self, range: Utf16Range) -> Option<String> {
        let range = CFRange {
            location: isize::try_from(range.start).ok()?,
            length: isize::try_from(range.len).ok()?,
        };
        let parameter =
            unsafe { AXValueCreate(AX_VALUE_CF_RANGE, (&range as *const CFRange).cast()) };
        if parameter.is_null() {
            return None;
        }
        let parameter = unsafe { CFType::wrap_under_create_rule(parameter) };
        let name = CFString::new("AXStringForRange");
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyParameterizedAttributeValue(
                self.raw(),
                name.as_concrete_TypeRef(),
                parameter.as_CFTypeRef(),
                &mut value,
            )
        };
        if status != AX_SUCCESS || value.is_null() {
            return None;
        }
        unsafe { CFType::wrap_under_create_rule(value) }
            .downcast::<CFString>()
            .map(|text| text.to_string())
    }

    pub fn children(&self) -> Vec<Element> {
        let Some(array) = self
            .attribute("AXChildren")
            .and_then(|value| value.downcast::<CFArray<*const c_void>>())
        else {
            return Vec::new();
        };
        array
            .get_all_values()
            .into_iter()
            .filter_map(|raw| {
                let value = unsafe { CFType::wrap_under_get_rule(raw as CFTypeRef) };
                Self::from_value(value)
            })
            .collect()
    }

    pub fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        (unsafe { AXUIElementGetPid(self.raw(), &mut pid) } == AX_SUCCESS && pid > 0).then_some(pid)
    }

    pub fn is_secure(&self) -> bool {
        ["AXRole", "AXSubrole"]
            .iter()
            .any(|name| self.string(name).as_deref() == Some(SECURE_TEXT_FIELD))
    }
}

/// The frontmost app other than Murmur, unless it is a password manager:
/// its pid, bundle identifier and application element.
pub fn frontmost_app() -> Option<(i32, String, Element)> {
    let application = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = application.processIdentifier();
    if pid <= 0 || u32::try_from(pid).ok() == Some(std::process::id()) {
        return None;
    }
    let bundle_id = application
        .bundleIdentifier()
        .map(|id| id.to_string())
        .unwrap_or_default();
    if is_password_app(&bundle_id) {
        return None;
    }
    Some((pid, bundle_id, Element::application(pid)?))
}

/// The keyboard focus anywhere on screen, when it is a readable, non-secure
/// element of an app that is not a password manager.
pub fn focused_text_element() -> Option<Element> {
    let (pid, _, _) = frontmost_app()?;
    let focused = Element::system_wide()?.element("AXFocusedUIElement")?;
    (focused.pid() == Some(pid) && !focused.is_secure()).then_some(focused)
}

/// The keyboard focus inside the app `pid`: the system-wide focus when it
/// belongs to that app, else the app's own, which some apps answer only
/// there. A secure field is returned as well; its reader must check.
pub fn focus_in(pid: i32, app: &Element) -> Option<Element> {
    Element::system_wide()
        .and_then(|system| system.element("AXFocusedUIElement"))
        .filter(|focused| focused.pid() == Some(pid))
        .or_else(|| app.element("AXFocusedUIElement"))
}

/// What holds the keyboard when a dictation ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusedField {
    TextField,
    /// A list, a button, a PDF page: nothing that takes typed text.
    NoTextField,
    /// Accessibility cannot tell. Electron apps without their accessibility
    /// tree report no focus at all, and pasting there works.
    Unknown,
}

/// The role of the focused element and whether it takes text input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusedElement {
    pub role: Option<String>,
    pub subrole: Option<String>,
    pub accepts_text: bool,
}

/// Any one of these marks an element that takes text, whatever its role,
/// such as a contenteditable web element reported as AXGroup.
const TEXT_INPUT_ATTRIBUTES: [&str; 3] = [
    "AXSelectedTextRange",
    "AXInsertionPointLineNumber",
    "AXEditableAncestor",
];

const TEXT_ROLES: [&str; 5] = [
    "AXTextField",
    "AXTextArea",
    "AXComboBox",
    SECURE_TEXT_FIELD,
    "AXSearchField",
];

/// Tables, rows and cells are left out: a selected spreadsheet cell takes a
/// paste without exposing any text attribute.
const NON_TEXT_ROLES: [&str; 15] = [
    "AXList",
    "AXOutline",
    "AXBrowser",
    "AXScrollArea",
    "AXImage",
    "AXButton",
    "AXCheckBox",
    "AXRadioButton",
    "AXPopUpButton",
    "AXMenuButton",
    "AXSlider",
    "AXTabGroup",
    "AXStaticText",
    "AXLayoutArea",
    "AXToolbar",
];

/// Finder's desktop answers AXSelectedTextRange and AXInsertionPointLineNumber
/// without listing them, though nothing on it takes text.
const NON_TEXT_SUBROLES: [&str; 1] = ["AXDesktop"];

/// Only a role known to take no text is `NoTextField`; containers such as
/// AXGroup, AXWebArea or AXWindow may hold an editor and stay `Unknown`.
pub fn classify_focus(focused: Option<FocusedElement>) -> FocusedField {
    let Some(focused) = focused else {
        return FocusedField::Unknown;
    };
    let role = focused.role.as_deref();
    let subrole = focused.subrole.as_deref();
    let in_table = |table: &[&str], name: Option<&str>| name.is_some_and(|n| table.contains(&n));
    if in_table(&NON_TEXT_SUBROLES, subrole) {
        FocusedField::NoTextField
    } else if focused.accepts_text || in_table(&TEXT_ROLES, role) || in_table(&TEXT_ROLES, subrole)
    {
        FocusedField::TextField
    } else if in_table(&NON_TEXT_ROLES, role) {
        FocusedField::NoTextField
    } else {
        FocusedField::Unknown
    }
}

/// Reads the keyboard focus of the frontmost app. Only the role and the
/// presence of text attributes are read, never the text, and password
/// managers are left alone.
pub fn keyboard_focus() -> FocusedField {
    let focused = focused_element();
    let field = classify_focus(focused.clone());
    log::debug!("Keyboard focus {focused:?} reads as {field:?}");
    field
}

fn focused_element() -> Option<FocusedElement> {
    let (pid, _, _) = frontmost_app()?;
    let focused = Element::system_wide()?.element("AXFocusedUIElement")?;
    (focused.pid() == Some(pid)).then(|| FocusedElement {
        role: focused.string("AXRole"),
        subrole: focused.string("AXSubrole"),
        accepts_text: TEXT_INPUT_ATTRIBUTES
            .iter()
            .any(|attribute| focused.has(attribute)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(role: &str, accepts_text: bool) -> Option<FocusedElement> {
        Some(FocusedElement {
            role: Some(role.to_string()),
            subrole: None,
            accepts_text,
        })
    }

    #[test]
    fn a_known_password_app_matches_in_any_case() {
        for bundle_id in [
            "com.1password.1password",
            "COM.APPLE.PASSWORDS",
            "com.bitwarden.desktop",
        ] {
            assert!(is_password_app(bundle_id), "{bundle_id}");
        }
    }

    #[test]
    fn other_bundle_ids_are_no_password_app() {
        for bundle_id in [
            "com.apple.Safari",
            "",
            "com.1password",
            "com.1password.1password.helper",
        ] {
            assert!(!is_password_app(bundle_id), "{bundle_id:?}");
        }
    }

    #[test]
    fn an_editable_element_is_a_text_field() {
        for (focused, why) in [
            (element("AXTextArea", false), "text area"),
            (element("AXTextField", false), "text field"),
            (element("AXComboBox", false), "combo box"),
            (element("AXGroup", true), "contenteditable group"),
            (element("AXWebArea", true), "editable web area"),
            (element("AXList", true), "list that takes text"),
            (element(SECURE_TEXT_FIELD, false), "secure field"),
            (
                Some(FocusedElement {
                    role: Some("AXTextField".to_string()),
                    subrole: Some("AXSearchField".to_string()),
                    accepts_text: false,
                }),
                "search field",
            ),
            (
                Some(FocusedElement {
                    role: None,
                    subrole: Some(SECURE_TEXT_FIELD.to_string()),
                    accepts_text: false,
                }),
                "secure subrole",
            ),
        ] {
            assert_eq!(classify_focus(focused), FocusedField::TextField, "{why}");
        }
    }

    #[test]
    fn a_non_editable_element_is_no_text_field() {
        for role in ["AXList", "AXScrollArea", "AXButton", "AXLayoutArea"] {
            assert_eq!(
                classify_focus(element(role, false)),
                FocusedField::NoTextField,
                "{role}"
            );
        }
    }

    /// Roles the probe in #197 read from Finder and System Settings.
    #[test]
    fn finder_and_settings_focus_is_no_text_field() {
        for (role, subrole, accepts_text, why) in [
            ("AXGroup", Some("AXDesktop"), true, "Finder desktop"),
            (
                "AXScrollArea",
                Some("AXDesktop"),
                false,
                "Finder desktop area",
            ),
            (
                "AXOutline",
                None,
                false,
                "Finder window or Settings sidebar",
            ),
        ] {
            let focused = FocusedElement {
                role: Some(role.to_string()),
                subrole: subrole.map(str::to_string),
                accepts_text,
            };
            assert_eq!(
                classify_focus(Some(focused)),
                FocusedField::NoTextField,
                "{why}"
            );
        }
    }

    #[test]
    fn an_unreadable_or_container_focus_is_unknown() {
        assert_eq!(classify_focus(None), FocusedField::Unknown);
        assert_eq!(
            classify_focus(Some(FocusedElement::default())),
            FocusedField::Unknown
        );
        for role in [
            "AXGroup",
            "AXWebArea",
            "AXWindow",
            "AXApplication",
            "AXTable",
            "AXCell",
            "AXNew",
        ] {
            assert_eq!(
                classify_focus(element(role, false)),
                FocusedField::Unknown,
                "{role}"
            );
        }
    }
}
