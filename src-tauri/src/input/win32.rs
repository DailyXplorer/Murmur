//! Keyboard input on Windows. `SendInput` cannot address a process: it goes
//! to whichever window has the keyboard focus. Each chord is therefore sent
//! only while the foreground window still belongs to the target process,
//! checked right before injection, on top of the caller's own revalidation.

use super::TargetedModifier;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    VK_CONTROL, VK_LWIN, VK_RETURN, VK_RWIN, VK_V,
};

/// One keystroke to synthesize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stroke {
    Key(VIRTUAL_KEY),
    /// A UTF-16 code unit typed through `KEYEVENTF_UNICODE`.
    Unit(u16),
}

fn keyboard_input(stroke: Stroke, key_up: bool) -> INPUT {
    let (virtual_key, scan, mut flags) = match stroke {
        // SAFETY: plain lookup in the active keyboard layout.
        Stroke::Key(key) => (
            key,
            unsafe { MapVirtualKeyW(u32::from(key.0), MAPVK_VK_TO_VSC) } as u16,
            KEYBD_EVENT_FLAGS(0),
        ),
        Stroke::Unit(unit) => (VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE),
    };
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    // SAFETY: `inputs` is a live slice of initialized keyboard INPUTs.
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!(
            "Windows accepted {sent} of {} keyboard events",
            inputs.len()
        ))
    }
}

fn require_foreground(pid: i32) -> Result<(), String> {
    let foreground = crate::foreground::window().map(|window| window.pid as i32);
    if foreground == Some(pid) {
        Ok(())
    } else {
        Err("the target app is no longer in front".to_string())
    }
}

/// Windows has no Command key; Control is its shortcut modifier.
fn modifier_key(modifier: TargetedModifier) -> VIRTUAL_KEY {
    match modifier {
        TargetedModifier::Command | TargetedModifier::Control => VK_CONTROL,
    }
}

fn is_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: plain query of the asynchronous key state.
    (unsafe { GetAsyncKeyState(i32::from(key.0)) } as u16 & 0x8000) != 0
}

/// Presses Ctrl+V without releasing Control. The caller schedules
/// `release_paste_modifier_to_pid` after the hardware-compatibility hold.
pub fn send_paste_to_pid(pid: i32) -> Result<(), String> {
    // With a Windows key still held, Ctrl+V would become the shell's
    // Win+Ctrl+V. The transcript then stays on the clipboard instead.
    if is_down(VK_LWIN) || is_down(VK_RWIN) {
        return Err("a Windows key is still held".to_string());
    }
    require_foreground(pid)?;
    send(&[
        keyboard_input(Stroke::Key(VK_CONTROL), false),
        keyboard_input(Stroke::Key(VK_V), false),
        keyboard_input(Stroke::Key(VK_V), true),
    ])
}

/// Releases the Control key of a paste chord. Keyboard state is global on
/// Windows, so the release goes out whatever window is now in front: a focus
/// change must not leave Control logically held.
pub fn release_paste_modifier_to_pid(_pid: i32) -> Result<(), String> {
    send(&[keyboard_input(Stroke::Key(VK_CONTROL), true)])
}

/// Sends Return, optionally with a modifier, to the target app.
pub fn send_return_to_pid(pid: i32, modifier: Option<TargetedModifier>) -> Result<(), String> {
    require_foreground(pid)?;
    let modifier = modifier.map(modifier_key);
    let mut inputs = Vec::with_capacity(4);
    inputs.extend(modifier.map(|key| keyboard_input(Stroke::Key(key), false)));
    inputs.push(keyboard_input(Stroke::Key(VK_RETURN), false));
    inputs.push(keyboard_input(Stroke::Key(VK_RETURN), true));
    inputs.extend(modifier.map(|key| keyboard_input(Stroke::Key(key), true)));
    send(&inputs)
}

/// The keystrokes that type `text`: line breaks as Return, which apps treat
/// as a new line where a typed control character is ignored, and everything
/// else as UTF-16 units.
fn text_strokes(text: &str) -> Vec<Stroke> {
    text.replace("\r\n", "\n")
        .chars()
        .flat_map(|character| match character {
            '\n' | '\r' => vec![Stroke::Key(VK_RETURN)],
            other => {
                let mut units = [0u16; 2];
                other
                    .encode_utf16(&mut units)
                    .iter()
                    .map(|unit| Stroke::Unit(*unit))
                    .collect()
            }
        })
        .collect()
}

/// Types `text` into the target app, in one `SendInput` call so no other
/// input can interleave with it.
pub fn send_text_to_pid(pid: i32, text: &str) -> Result<(), String> {
    require_foreground(pid)?;
    let inputs: Vec<INPUT> = text_strokes(text)
        .into_iter()
        .flat_map(|stroke| [keyboard_input(stroke, false), keyboard_input(stroke, true)])
        .collect();
    send(&inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_typed_as_utf16_units_with_line_breaks_as_return() {
        assert_eq!(
            text_strokes("é\r\n😀\nx"),
            vec![
                Stroke::Unit(0xE9),
                Stroke::Key(VK_RETURN),
                Stroke::Unit(0xD83D),
                Stroke::Unit(0xDE00),
                Stroke::Key(VK_RETURN),
                Stroke::Unit(u16::from(b'x')),
            ]
        );
    }

    #[test]
    fn every_shortcut_modifier_is_control_on_windows() {
        assert_eq!(modifier_key(TargetedModifier::Command), VK_CONTROL);
        assert_eq!(modifier_key(TargetedModifier::Control), VK_CONTROL);
    }

    #[test]
    fn a_unicode_unit_carries_no_virtual_key() {
        let input = keyboard_input(Stroke::Unit(0x263A), true);
        // SAFETY: keyboard_input always fills the `ki` member.
        let key = unsafe { input.Anonymous.ki };
        assert_eq!(key.wVk, VIRTUAL_KEY(0));
        assert_eq!(key.wScan, 0x263A);
        assert_eq!(key.dwFlags, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
    }
}
