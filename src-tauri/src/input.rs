#[cfg(target_os = "macos")]
use core_graphics::event::{CGEvent, CGEventFlags, CGEventType, KeyCode};
#[cfg(target_os = "macos")]
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
#[cfg(target_os = "macos")]
use enigo::{Enigo, Mouse, Settings};
#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use std::time::Duration;
#[cfg(target_os = "macos")]
use tauri::{AppHandle, Manager};

#[cfg(target_os = "windows")]
mod win32;
#[cfg(target_os = "windows")]
pub use win32::{
    release_paste_modifier_to_pid, send_paste_to_pid, send_return_to_pid, send_text_to_pid,
};

#[cfg(target_os = "macos")]
mod macos {
    use log::{debug, warn};
    use std::ffi::c_void;

    type TisInputSourceRef = *const c_void;
    type CfDataRef = *const c_void;
    type CfStringRef = *const c_void;

    // kVK_ANSI_V, the fallback when macOS cannot expose the active layout.
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

    /// What the current layout types on each virtual key code while Command
    /// is held, indexed by key code. Including Command is important:
    /// non-Latin layouts commonly map Cmd shortcuts to their ANSI
    /// equivalents, while standard Dvorak does not.
    ///
    /// TIS APIs must run on the main thread. Murmur's paste path already enters
    /// through `AppHandle::run_on_main_thread` before reaching this function.
    pub(super) fn command_chars() -> Result<Vec<Option<char>>, String> {
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
        Ok((0..KEYCODE_COUNT)
            .map(|keycode| {
                let mut dead_key_state = 0;
                let mut chars = [0_u16; 4];
                let mut length = 0_usize;

                // SAFETY: layout points to valid UCKeyboardLayout bytes while
                // source is retained. All output pointers reference initialized
                // local storage of the declared sizes.
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

                if status == 0 && length == 1 {
                    char::from_u32(u32::from(chars[0]))
                } else {
                    None
                }
            })
            .collect())
    }

    /// Resolves the physical key that macOS interprets as `v` while Command is
    /// held.
    fn resolve_command_v_keycode() -> Result<u16, String> {
        command_chars()?
            .iter()
            .position(|&typed| typed == Some('v'))
            .map(|keycode| keycode as u16)
            .ok_or_else(|| "could not map Cmd+V in the current macOS keyboard layout".to_string())
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

/// What the current keyboard layout types on each virtual key code while
/// Command is held, indexed by key code. Call it on the main thread.
#[cfg(target_os = "macos")]
pub fn command_key_chars() -> Result<Vec<Option<char>>, String> {
    macos::command_chars()
}

#[derive(Clone, Copy)]
pub enum TargetedModifier {
    Command,
    Control,
}

#[cfg(target_os = "macos")]
fn modifier_event(modifier: TargetedModifier) -> (u16, CGEventFlags) {
    match modifier {
        TargetedModifier::Command => (KeyCode::COMMAND, CGEventFlags::CGEventFlagCommand),
        TargetedModifier::Control => (KeyCode::CONTROL, CGEventFlags::CGEventFlagControl),
    }
}

/// Pause between the events of one synthetic chord. Hardware delivers a
/// modifier change and the key that follows it some milliseconds apart, and
/// remote-desktop and VM clients that track modifiers from their own event
/// stream need that gap to see the modifier before the key.
#[cfg(target_os = "macos")]
const CHORD_EVENT_GAP: Duration = Duration::from_millis(15);

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChordEventKind {
    /// A modifier press or release. A keyboard sends this as a
    /// flags-changed event, never as a key-down or key-up, and clients that
    /// follow the modifier state from their own events ignore the latter.
    ModifierChange,
    KeyDown,
    KeyUp,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChordEvent {
    kind: ChordEventKind,
    keycode: u16,
    /// Modifier flags the event carries once it is applied.
    flags: CGEventFlags,
    /// Time to wait after the previous event of the chord.
    gap: Duration,
}

#[cfg(target_os = "macos")]
fn modifier_change_event(keycode: u16, flags: CGEventFlags, gap: Duration) -> ChordEvent {
    ChordEvent {
        kind: ChordEventKind::ModifierChange,
        keycode,
        flags,
        gap,
    }
}

/// The events of `modifier` + `keycode`, in posting order. With
/// `release_modifier` false the modifier stays down and the caller schedules
/// `release_paste_modifier_to_pid` later.
#[cfg(target_os = "macos")]
fn chord_plan(
    modifier: Option<TargetedModifier>,
    keycode: u16,
    release_modifier: bool,
) -> Vec<ChordEvent> {
    let modifier = modifier.map(modifier_event);
    let flags = modifier.map_or(CGEventFlags::CGEventFlagNull, |(_, flags)| flags);
    let mut plan = Vec::with_capacity(4);
    if let Some((modifier_keycode, modifier_flags)) = modifier {
        plan.push(modifier_change_event(
            modifier_keycode,
            modifier_flags,
            Duration::ZERO,
        ));
    }
    plan.push(ChordEvent {
        kind: ChordEventKind::KeyDown,
        keycode,
        flags,
        gap: if plan.is_empty() {
            Duration::ZERO
        } else {
            CHORD_EVENT_GAP
        },
    });
    plan.push(ChordEvent {
        kind: ChordEventKind::KeyUp,
        keycode,
        flags,
        // A bare key has no modifier to wait for: it goes out as before.
        gap: if modifier.is_some() {
            CHORD_EVENT_GAP
        } else {
            Duration::ZERO
        },
    });
    if let (Some((modifier_keycode, _)), true) = (modifier, release_modifier) {
        plan.push(modifier_change_event(
            modifier_keycode,
            CGEventFlags::CGEventFlagNull,
            CHORD_EVENT_GAP,
        ));
    }
    plan
}

#[cfg(target_os = "macos")]
fn build_event(source: &CGEventSource, planned: &ChordEvent) -> Result<CGEvent, String> {
    let keydown = planned.kind != ChordEventKind::KeyUp;
    let event = CGEvent::new_keyboard_event(source.clone(), planned.keycode, keydown)
        .map_err(|_| "Failed to create targeted keyboard event".to_string())?;
    if planned.kind == ChordEventKind::ModifierChange {
        event.set_type(CGEventType::FlagsChanged);
    }
    event.set_flags(planned.flags);
    Ok(event)
}

#[cfg(target_os = "macos")]
fn keyboard_event(
    source: &CGEventSource,
    keycode: u16,
    keydown: bool,
    flags: CGEventFlags,
) -> Result<CGEvent, String> {
    let kind = if keydown {
        ChordEventKind::KeyDown
    } else {
        ChordEventKind::KeyUp
    };
    build_event(
        source,
        &ChordEvent {
            kind,
            keycode,
            flags,
            gap: Duration::ZERO,
        },
    )
}

/// A CoreGraphics event that waits to be posted by the task that owns it.
#[cfg(target_os = "macos")]
struct PendingChordEvent {
    gap: Duration,
    event: CGEvent,
}

// SAFETY: a CGEvent is a retained CoreFoundation object. These are fully
// built before the move, never touched again by the creating thread, and
// `CGEventPostToPid` may run on any thread.
#[cfg(target_os = "macos")]
unsafe impl Send for PendingChordEvent {}

/// Posts the events of the plan that need no wait now and the rest from a
/// task, one gap apart, so neither the caller nor the main thread waits. Every event is
/// built first: a build failure posts nothing. The tail keeps going when a
/// post cannot be observed, so a trailing modifier release is never skipped.
#[cfg(target_os = "macos")]
fn post_chord_to_pid(pid: i32, plan: &[ChordEvent]) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "Failed to create targeted keyboard event source".to_string())?;
    let pending = plan
        .iter()
        .map(|planned| {
            build_event(&source, planned).map(|event| PendingChordEvent {
                gap: planned.gap,
                event,
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter();
    // Events with no gap in front of them go out inline, so a bare Return is
    // posted whole before this returns and spawns nothing.
    let mut rest = Vec::new();
    for step in pending {
        if rest.is_empty() && step.gap.is_zero() {
            step.event.post_to_pid(pid);
        } else {
            rest.push(step);
        }
    }
    if !rest.is_empty() {
        tauri::async_runtime::spawn(async move {
            for step in rest {
                tokio::time::sleep(step.gap).await;
                step.event.post_to_pid(pid);
            }
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn send_key_chord_to_pid(
    pid: i32,
    keycode: u16,
    modifier: Option<TargetedModifier>,
) -> Result<(), String> {
    post_chord_to_pid(pid, &chord_plan(modifier, keycode, true))
}

/// Posts Command+V without releasing Command. The caller must schedule
/// `release_paste_modifier_to_pid` after the hardware-compatibility hold,
/// which is longer than the gaps that space the three events posted here.
/// Both calls must run on the macOS main thread.
#[cfg(target_os = "macos")]
pub fn send_paste_to_pid(pid: i32) -> Result<(), String> {
    let keycode = macos::command_v_keycode();
    post_chord_to_pid(
        pid,
        &chord_plan(Some(TargetedModifier::Command), keycode, false),
    )
}

/// Releases the Command key for a previously posted targeted paste chord.
/// This deliberately does not revalidate focus: a focus switch must not leave
/// Command logically held in the process that received the original key-down.
#[cfg(target_os = "macos")]
pub fn release_paste_modifier_to_pid(pid: i32) -> Result<(), String> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| "Failed to create targeted keyboard event source".to_string())?;
    let event = build_event(
        &source,
        &modifier_change_event(
            KeyCode::COMMAND,
            CGEventFlags::CGEventFlagNull,
            Duration::ZERO,
        ),
    )?;
    event.post_to_pid(pid);
    Ok(())
}

/// Sends Return, optionally with a modifier, directly to one process.
#[cfg(target_os = "macos")]
pub fn send_return_to_pid(pid: i32, modifier: Option<TargetedModifier>) -> Result<(), String> {
    send_key_chord_to_pid(pid, KeyCode::RETURN, modifier)
}

#[cfg(target_os = "macos")]
const MAX_UNICODE_EVENT_UNITS: usize = 20;

#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
pub struct EnigoState(pub Mutex<Enigo>);

#[cfg(target_os = "macos")]
impl EnigoState {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Failed to initialize Enigo: {}", e))?;
        Ok(Self(Mutex::new(enigo)))
    }
}

/// Get the current mouse cursor position using the managed Enigo instance.
/// Returns None if the state is not available or if getting the location fails.
#[cfg(target_os = "macos")]
pub fn get_cursor_position(app_handle: &AppHandle) -> Option<(i32, i32)> {
    let enigo_state = app_handle.try_state::<EnigoState>()?;
    let enigo = enigo_state.0.lock().ok()?;
    enigo.location().ok()
}

#[cfg(test)]
mod targeted_input_tests {
    #[test]
    fn targeted_input_does_not_block_for_modifier_holds() {
        let forbidden = ["thread", "::sleep"].concat();
        assert!(!include_str!("input.rs").contains(&forbidden));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_targeted_input_tests {
    use super::*;
    use core_graphics::event::EventField;

    const V: u16 = 9;

    fn shape(plan: &[ChordEvent]) -> Vec<(ChordEventKind, u16, CGEventFlags)> {
        plan.iter()
            .map(|event| (event.kind, event.keycode, event.flags))
            .collect()
    }

    #[test]
    fn paste_plan_presses_command_as_a_flags_change_then_taps_v() {
        let plan = chord_plan(Some(TargetedModifier::Command), V, false);
        let command = CGEventFlags::CGEventFlagCommand;
        assert_eq!(
            shape(&plan),
            vec![
                (ChordEventKind::ModifierChange, KeyCode::COMMAND, command),
                (ChordEventKind::KeyDown, V, command),
                (ChordEventKind::KeyUp, V, command),
            ]
        );
    }

    #[test]
    fn paste_plan_spaces_events_and_leaves_command_to_the_caller() {
        let plan = chord_plan(Some(TargetedModifier::Command), V, false);
        let gaps: Vec<Duration> = plan.iter().map(|event| event.gap).collect();
        assert_eq!(gaps, vec![Duration::ZERO, CHORD_EVENT_GAP, CHORD_EVENT_GAP]);
        assert_eq!(CHORD_EVENT_GAP, Duration::from_millis(15));
        // The release is scheduled by the caller after the longer hold.
        assert!(CHORD_EVENT_GAP * 2 < Duration::from_millis(100));
    }

    #[test]
    fn modifier_return_plan_releases_the_modifier_last_with_cleared_flags() {
        for (modifier, keycode, flag) in [
            (
                TargetedModifier::Command,
                KeyCode::COMMAND,
                CGEventFlags::CGEventFlagCommand,
            ),
            (
                TargetedModifier::Control,
                KeyCode::CONTROL,
                CGEventFlags::CGEventFlagControl,
            ),
        ] {
            let plan = chord_plan(Some(modifier), KeyCode::RETURN, true);
            assert_eq!(
                shape(&plan),
                vec![
                    (ChordEventKind::ModifierChange, keycode, flag),
                    (ChordEventKind::KeyDown, KeyCode::RETURN, flag),
                    (ChordEventKind::KeyUp, KeyCode::RETURN, flag),
                    (
                        ChordEventKind::ModifierChange,
                        keycode,
                        CGEventFlags::CGEventFlagNull
                    ),
                ]
            );
            assert!(plan[1..].iter().all(|event| event.gap == CHORD_EVENT_GAP));
        }
    }

    #[test]
    fn plain_return_plan_has_no_modifier_events() {
        let plan = chord_plan(None, KeyCode::RETURN, true);
        let none = CGEventFlags::CGEventFlagNull;
        assert_eq!(
            shape(&plan),
            vec![
                (ChordEventKind::KeyDown, KeyCode::RETURN, none),
                (ChordEventKind::KeyUp, KeyCode::RETURN, none),
            ]
        );
        assert!(plan.iter().all(|event| event.gap.is_zero()));
    }

    #[test]
    fn built_events_carry_the_planned_type_keycode_and_flags() {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).unwrap();
        let plan = chord_plan(Some(TargetedModifier::Command), V, true);
        let types: Vec<u32> = plan
            .iter()
            .map(|planned| {
                let event = build_event(&source, planned).unwrap();
                assert_eq!(
                    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
                    i64::from(planned.keycode)
                );
                assert_eq!(event.get_flags(), planned.flags);
                event.get_type() as u32
            })
            .collect();
        assert_eq!(
            types,
            vec![
                CGEventType::FlagsChanged as u32,
                CGEventType::KeyDown as u32,
                CGEventType::KeyUp as u32,
                CGEventType::FlagsChanged as u32,
            ]
        );
    }

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
