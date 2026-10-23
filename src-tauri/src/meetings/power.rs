//! Keeps the Mac from sleeping on its own while a meeting records. Closing
//! the lid still puts it to sleep: no public API prevents that without an
//! external display.

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use log::warn;

type AssertionId = u32;

/// `kIOPMAssertionLevelOn`.
const LEVEL_ON: u32 = 255;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        kind: CFStringRef,
        level: u32,
        name: CFStringRef,
        id: *mut AssertionId,
    ) -> i32;
    fn IOPMAssertionRelease(id: AssertionId) -> i32;
}

/// Holds `kIOPMAssertionTypePreventUserIdleSystemSleep` until dropped. The
/// screen still turns off, and macOS drops the assertion if Murmur exits.
pub struct AwakeGuard(AssertionId);

impl AwakeGuard {
    /// `reason` is what `pmset -g assertions` shows.
    pub fn hold(reason: &str) -> Option<Self> {
        let kind = CFString::from_static_string("PreventUserIdleSystemSleep");
        let name = CFString::new(reason);
        let mut id = 0;
        // SAFETY: both strings outlive the call, which only writes `id`.
        let result = unsafe {
            IOPMAssertionCreateWithName(
                kind.as_concrete_TypeRef(),
                LEVEL_ON,
                name.as_concrete_TypeRef(),
                &mut id,
            )
        };
        if result != 0 {
            warn!("macOS refused to keep the Mac awake: IOReturn {result:#x}");
            return None;
        }
        Some(Self(id))
    }
}

impl Drop for AwakeGuard {
    fn drop(&mut self) {
        // SAFETY: the id came from a successful create and is released once.
        let result = unsafe { IOPMAssertionRelease(self.0) };
        if result != 0 {
            warn!("Could not let the Mac sleep again: IOReturn {result:#x}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(reason: &str) -> bool {
        let output = std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).contains(reason)
    }

    #[test]
    fn the_mac_is_kept_awake_until_the_guard_is_dropped() {
        let reason = format!("Murmur power test {}", std::process::id());
        let guard = AwakeGuard::hold(&reason).unwrap();
        assert!(listed(&reason));
        drop(guard);
        assert!(!listed(&reason));
    }
}
