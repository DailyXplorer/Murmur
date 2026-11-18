//! The mute of the default output device through the Core Audio HAL. It is
//! the switch `set volume output muted` flips, read and written without the
//! 0.13 s or so an `osascript` process takes to start.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

type AudioObjectId = u32;

/// `kAudioObjectSystemObject`.
const SYSTEM_OBJECT: AudioObjectId = 1;
/// `kAudioObjectUnknown`, the default output of a Mac with no output device.
const UNKNOWN_OBJECT: AudioObjectId = 0;
const NO_ERROR: i32 = 0;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

/// `kAudioHardwarePropertyDefaultOutputDevice`, global scope, main element.
const DEFAULT_OUTPUT_DEVICE: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    selector: four_cc(b"dOut"),
    scope: four_cc(b"glob"),
    element: 0,
};

/// `kAudioDevicePropertyMute`, output scope, main element.
const OUTPUT_MUTE: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    selector: four_cc(b"mute"),
    scope: four_cc(b"outp"),
    element: 0,
};

const fn four_cc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectHasProperty(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
    ) -> u8;
    fn AudioObjectIsPropertySettable(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        settable: *mut u8,
    ) -> i32;
    fn AudioObjectGetPropertyData(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioObjectSetPropertyData(
        object: AudioObjectId,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;
}

fn read_u32(object: AudioObjectId, address: &AudioObjectPropertyAddress) -> Option<u32> {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `address`, `size` and `value` outlive the call, which writes at
    // most `size` bytes, the size of `value`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            address,
            0,
            ptr::null(),
            &mut size,
            ptr::from_mut(&mut value).cast(),
        )
    };
    (status == NO_ERROR && size == size_of::<u32>() as u32).then_some(value)
}

/// The default output device, when it has a mute of its own. A display or an
/// aggregate device often has none.
fn output_with_mute() -> Option<AudioObjectId> {
    let device = read_u32(SYSTEM_OBJECT, &DEFAULT_OUTPUT_DEVICE)?;
    if device == UNKNOWN_OBJECT {
        return None;
    }
    // SAFETY: the address is a constant, which the call only reads.
    let has_mute = unsafe { AudioObjectHasProperty(device, &OUTPUT_MUTE) } != 0;
    has_mute.then_some(device)
}

/// Whether the default output is muted. `None` when Core Audio cannot tell.
pub(super) fn muted() -> Option<bool> {
    read_u32(output_with_mute()?, &OUTPUT_MUTE).map(|muted| muted != 0)
}

/// Mutes or unmutes the default output. `false` when Core Audio could not.
#[must_use]
pub(super) fn set_muted(mute: bool) -> bool {
    let Some(device) = output_with_mute() else {
        return false;
    };
    let mut settable: u8 = 0;
    // SAFETY: the address is a constant and `settable` outlives the call,
    // which writes one byte to it.
    let status = unsafe { AudioObjectIsPropertySettable(device, &OUTPUT_MUTE, &mut settable) };
    if status != NO_ERROR || settable == 0 {
        return false;
    }
    let value = u32::from(mute);
    // SAFETY: `value` outlives the call, which reads the four bytes it is
    // told about.
    let status = unsafe {
        AudioObjectSetPropertyData(
            device,
            &OUTPUT_MUTE,
            0,
            ptr::null(),
            size_of::<u32>() as u32,
            ptr::from_ref(&value).cast(),
        )
    };
    status == NO_ERROR
}
