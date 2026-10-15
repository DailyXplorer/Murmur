//! Read-only Accessibility queries of other apps: the focused text field and
//! the text of the frontmost window. Needs the Accessibility permission
//! Murmur already holds for pasting; no screenshots, no Screen Recording.
//! Secure text fields and password managers are never read.

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeID, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;

type AXUIElementRef = CFTypeRef;
const AX_SUCCESS: i32 = 0;
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
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
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
