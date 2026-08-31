use core_graphics::event::{CGEvent, CGEventFlags, KeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use enigo::{Enigo, Mouse, Settings};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

mod macos {
    use log::{debug, warn};
    use std::ffi::c_void;

    type TisInputSourceRef = *const c_void;
    type CfDataRef = *const c_void;
    type CfStringRef = *const c_void;

    // kVK_ANSI_V. This is the behavior Murmur used before layout-aware
    // resolution and remains the safest fallback if macOS cannot expose the
    // active layout.
    const ANSI_V_KEYCODE: u16 = 9;
    const KEYCODE_COUNT: u16 = 128;
    const UC_KEY_ACTION_DISPLAY: u16 = 3;
    const UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: u32 = 1;
    // Carbon's cmdKey is bit 8. UCKeyTranslate expects Carbon modifiers shifted
    // right by 8, so Command is represented by bit 0 here.
    const COMMAND_MODIFIER_STATE: u32 = 1;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn TISCopyCurrentKeyboardLayoutInputSource() -> TisInputSourceRef;
        fn TISGetInputSourceProperty(
            input_source: TisInputSourceRef,
            property_key: CfStringRef,
        ) -> CfDataRef;
        static kTISPropertyUnicodeKeyLayoutData: CfStringRef;
        fn UCKeyTranslate(
            key_layout: *const u8,
            virtual_key_code: u16,
            key_action: u16,
            modifier_key_state: u32,
            keyboard_type: u32,
            key_translate_options: u32,
            dead_key_state: *mut u32,
            max_string_length: usize,
            actual_string_length: *mut usize,
            unicode_string: *mut u16,
        ) -> i32;
        fn LMGetKbdType() -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataGetBytePtr(data: CfDataRef) -> *const u8;
        fn CFRelease(value: *const c_void);
    }

    struct InputSource(TisInputSourceRef);

    impl Drop for InputSource {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: TISCopyCurrentKeyboardLayoutInputSource returned this
                // retained reference, so this balances that ownership.
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn find_keycode(mut matches: impl FnMut(u16) -> bool) -> Option<u16> {
        (0..KEYCODE_COUNT).find(|&keycode| matches(keycode))
    }

    /// Resolves the physical key that macOS interprets as `v` while Command is
    /// held. Including Command is important: non-Latin layouts commonly map
    /// Cmd shortcuts to their ANSI equivalents, while standard Dvorak does not.
    ///
    /// TIS APIs must run on the main thread. Murmur's paste path already enters
    /// through `AppHandle::run_on_main_thread` before reaching this function.
    fn resolve_command_v_keycode() -> Result<u16, String> {
        // SAFETY: This function is called on the macOS main thread. The returned
        // source follows the Create Rule and is released by InputSource::drop.
        let source = InputSource(unsafe { TISCopyCurrentKeyboardLayoutInputSource() });
        if source.0.is_null() {
            return Err("macOS returned no current keyboard layout input source".into());
        }

        // SAFETY: The source remains retained for the duration of the scan and
        // the property constant is provided by Carbon.
        let layout_data =
            unsafe { TISGetInputSourceProperty(source.0, kTISPropertyUnicodeKeyLayoutData) };
        if layout_data.is_null() {
            return Err("current macOS keyboard layout has no Unicode layout data".into());
        }

        // SAFETY: layout_data is a CFData owned by the retained input source and
        // remains valid until source is dropped after the scan.
        let layout = unsafe { CFDataGetBytePtr(layout_data) };
        if layout.is_null() {
            return Err("current macOS keyboard layout data is empty".into());
        }

        // SAFETY: LMGetKbdType has no arguments and returns the current physical
        // keyboard type used by UCKeyTranslate.
        let keyboard_type = unsafe { LMGetKbdType() } as u32;
        let keycode = find_keycode(|keycode| {
            let mut dead_key_state = 0;
            let mut chars = [0_u16; 4];
            let mut length = 0_usize;

            // SAFETY: layout points to valid UCKeyboardLayout bytes while source
            // is retained. All output pointers reference initialized local
            // storage of the declared sizes.
            let status = unsafe {
                UCKeyTranslate(
                    layout,
                    keycode,
                    UC_KEY_ACTION_DISPLAY,
                    COMMAND_MODIFIER_STATE,
                    keyboard_type,
                    UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK,
                    &mut dead_key_state,
                    chars.len(),
                    &mut length,
                    chars.as_mut_ptr(),
                )
            };

            status == 0 && length == 1 && chars[0] == u16::from(b'v')
        })
        .ok_or_else(|| "could not map Cmd+V in the current macOS keyboard layout".to_string())?;

        Ok(keycode)
    }

    pub(super) fn command_v_keycode() -> u16 {
        match resolve_command_v_keycode() {
            Ok(keycode) => {
                debug!("Resolved Cmd+V for the active macOS layout to keycode {keycode}");
                keycode
            }
            Err(error) => {
                warn!(
                    "Could not resolve Cmd+V for the active macOS layout ({error}); using ANSI V keycode {ANSI_V_KEYCODE}"
                );
                ANSI_V_KEYCODE
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum TargetedModifier {
    Command,
    Control,
}

fn modifier_event(modifier: TargetedModifier) -> (u16, CGEventFlags) {
    match modifier {
        TargetedModifier::Command => (KeyCode::COMMAND, CGEventFlags::CGEventFlagCommand),
        TargetedModifier::Control => (KeyCode::CONTROL, CGEventFlags::CGEventFlagControl),
    }
}

fn keyboard_event(
    source: &CGEventSource,
    keycode: u16,
    keydown: bool,
    flags: CGEventFlags,
) -> Result<CGEvent, String> {
    let event = CGEvent::new_keyboard_event(source.clone(), keycode, keydown)
        .map_err(|_| "Failed to create targeted keyboard event".to_string())?;
    event.set_flags(flags);
    Ok(event)
}

fn send_key_chord_to_pid(
    pid: i32,
    keycode: u16,
    modifier: Option<TargetedModifier>,
    hold_ms: u64,
) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "Failed to create targeted keyboard event source".to_string())?;
    let modifier_data = modifier.map(modifier_event);

    let flags = modifier_data
        .map(|(_, flags)| flags)
        .unwrap_or(CGEventFlags::CGEventFlagNull);
    let modifier_down = modifier_data
        .map(|(modifier_keycode, flags)| keyboard_event(&source, modifier_keycode, true, flags))
        .transpose()?;
    let key_down = keyboard_event(&source, keycode, true, flags)?;
    let key_up = keyboard_event(&source, keycode, false, flags)?;
    let modifier_up = modifier_data
        .map(|(modifier_keycode, _)| {
            keyboard_event(
                &source,
                modifier_keycode,
                false,
                CGEventFlags::CGEventFlagNull,
            )
        })
        .transpose()?;

    if let Some(event) = modifier_down {
        event.post_to_pid(pid);
    }
    key_down.post_to_pid(pid);
    key_up.post_to_pid(pid);

    if let Some(event) = modifier_up {
        std::thread::sleep(std::time::Duration::from_millis(hold_ms));
        event.post_to_pid(pid);
    }
    Ok(())
}

/// Sends Cmd+V directly to the process that was frontmost when the operation
/// began. Unlike a global HID post, a focus switch cannot redirect the chord.
pub fn send_paste_to_pid(pid: i32, hold_ms: u64) -> Result<(), String> {
    send_key_chord_to_pid(
        pid,
        macos::command_v_keycode(),
        Some(TargetedModifier::Command),
        hold_ms,
    )
}

/// Sends Return, optionally with a modifier, directly to one process.
pub fn send_return_to_pid(pid: i32, modifier: Option<TargetedModifier>) -> Result<(), String> {
    send_key_chord_to_pid(pid, KeyCode::RETURN, modifier, 0)
}

const MAX_UNICODE_EVENT_UNITS: usize = 20;

fn targeted_text_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_units = 0;

    for character in text.chars() {
        let character_units = character.len_utf16();
        if !current.is_empty() && current_units + character_units > MAX_UNICODE_EVENT_UNITS {
            chunks.push(std::mem::take(&mut current));
            current_units = 0;
        }

        // CGEventKeyboardSetUnicodeString ignores a payload that starts with
        // one of these controls. Enigo uses the same zero-width prefix
        // workaround; it keeps multiline legacy Direct mode functional.
        if current.is_empty() && matches!(character, '\n' | '\r' | '\t') {
            current.push('\u{200b}');
            current_units += 1;
        }
        current.push(character);
        current_units += character_units;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Posts Unicode text only to the captured process. Each CoreGraphics payload
/// is bounded to macOS's 20 UTF-16-unit limit, so no later chunk can leak to a
/// process that steals focus while a long transcription is being inserted.
pub fn send_text_to_pid(pid: i32, text: &str) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "Failed to create targeted text event source".to_string())?;
    for chunk in targeted_text_chunks(text) {
        let event = keyboard_event(&source, 0, true, CGEventFlags::CGEventFlagNull)?;
        event.set_string(&chunk);
        event.post_to_pid(pid);
    }
    Ok(())
}

/// Wrapper for Enigo to store in Tauri's managed state.
/// Enigo is wrapped in a Mutex since it requires mutable access.
pub struct EnigoState(pub Mutex<Enigo>);

impl EnigoState {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to initialize Enigo: {}", e))?;
        Ok(Self(Mutex::new(enigo)))
    }
}

/// Get the current mouse cursor position using the managed Enigo instance.
/// Returns None if the state is not available or if getting the location fails.
pub fn get_cursor_position(app_handle: &AppHandle) -> Option<(i32, i32)> {
    let enigo_state = app_handle.try_state::<EnigoState>()?;
    let enigo = enigo_state.0.lock().ok()?;
    enigo.location().ok()
}

#[cfg(test)]
mod targeted_input_tests {
    use super::*;

    #[test]
    fn unicode_chunks_respect_utf16_limit_without_splitting_scalars() {
        let text = format!("{}😀{}", "a".repeat(19), "b".repeat(19));
        let chunks = targeted_text_chunks(&text);
        assert_eq!(chunks.concat(), text);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.encode_utf16().count() <= MAX_UNICODE_EVENT_UNITS));
    }

    #[test]
    fn leading_controls_get_the_core_graphics_workaround() {
        for input in ["\nline", "\rline", "\tline"] {
            let chunks = targeted_text_chunks(input);
            assert!(chunks[0].starts_with('\u{200b}'));
            assert_eq!(chunks[0].trim_start_matches('\u{200b}'), input);
        }
    }
}
