//! What the active keyboard layout writes on the keys a shortcut names.
//!
//! macOS registers a shortcut on a physical key, named after what that key
//! types on a US keyboard: "ctrl+option+command+m" is the key left of ","
//! there, which is the "," key on a French AZERTY keyboard. Its label must
//! say ",".
//!
//! Windows registers a virtual key, which the layout places: "ctrl+1" fires
//! on the key whose virtual key is 1, which types "&" on AZERTY. The recorder
//! sees physical keys, so it also needs the name of the virtual key under
//! each of them ([`read_names`]).

use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// The keys whose character moves with the layout, by the name the shortcut
/// parser gives them, with the virtual key code global-hotkey registers.
#[cfg(target_os = "macos")]
const LAYOUT_KEYS: [(&str, u16); 47] = [
    ("a", 0x00),
    ("s", 0x01),
    ("d", 0x02),
    ("f", 0x03),
    ("h", 0x04),
    ("g", 0x05),
    ("z", 0x06),
    ("x", 0x07),
    ("c", 0x08),
    ("v", 0x09),
    ("b", 0x0b),
    ("q", 0x0c),
    ("w", 0x0d),
    ("e", 0x0e),
    ("r", 0x0f),
    ("y", 0x10),
    ("t", 0x11),
    ("1", 0x12),
    ("2", 0x13),
    ("3", 0x14),
    ("4", 0x15),
    ("6", 0x16),
    ("5", 0x17),
    ("=", 0x18),
    ("9", 0x19),
    ("7", 0x1a),
    ("-", 0x1b),
    ("8", 0x1c),
    ("0", 0x1d),
    ("]", 0x1e),
    ("o", 0x1f),
    ("u", 0x20),
    ("[", 0x21),
    ("i", 0x22),
    ("p", 0x23),
    ("l", 0x25),
    ("j", 0x26),
    ("'", 0x27),
    ("k", 0x28),
    (";", 0x29),
    ("\\", 0x2a),
    (",", 0x2b),
    ("/", 0x2c),
    ("n", 0x2d),
    ("m", 0x2e),
    (".", 0x2f),
    ("`", 0x32),
];

/// The keys whose character moves with the layout, by the name the shortcut
/// parser gives them, with the virtual key global-hotkey's `key_to_vk`
/// registers for that name.
#[cfg(target_os = "windows")]
const LAYOUT_KEYS: [(&str, u16); 47] = [
    ("a", VK_A.0),
    ("b", VK_B.0),
    ("c", VK_C.0),
    ("d", VK_D.0),
    ("e", VK_E.0),
    ("f", VK_F.0),
    ("g", VK_G.0),
    ("h", VK_H.0),
    ("i", VK_I.0),
    ("j", VK_J.0),
    ("k", VK_K.0),
    ("l", VK_L.0),
    ("m", VK_M.0),
    ("n", VK_N.0),
    ("o", VK_O.0),
    ("p", VK_P.0),
    ("q", VK_Q.0),
    ("r", VK_R.0),
    ("s", VK_S.0),
    ("t", VK_T.0),
    ("u", VK_U.0),
    ("v", VK_V.0),
    ("w", VK_W.0),
    ("x", VK_X.0),
    ("y", VK_Y.0),
    ("z", VK_Z.0),
    ("0", VK_0.0),
    ("1", VK_1.0),
    ("2", VK_2.0),
    ("3", VK_3.0),
    ("4", VK_4.0),
    ("5", VK_5.0),
    ("6", VK_6.0),
    ("7", VK_7.0),
    ("8", VK_8.0),
    ("9", VK_9.0),
    ("=", VK_OEM_PLUS.0),
    (",", VK_OEM_COMMA.0),
    ("-", VK_OEM_MINUS.0),
    (".", VK_OEM_PERIOD.0),
    (";", VK_OEM_1.0),
    ("/", VK_OEM_2.0),
    ("`", VK_OEM_3.0),
    ("[", VK_OEM_4.0),
    ("\\", VK_OEM_5.0),
    ("]", VK_OEM_6.0),
    ("'", VK_OEM_7.0),
];

/// The same keys as physical keys, and the key between left Shift and Z on
/// ISO keyboards: the name the recorder gives each one (what it types on a US
/// keyboard, else its key code), with its scan code.
#[cfg(target_os = "windows")]
const PHYSICAL_KEYS: [(&str, u16); 48] = [
    ("1", 0x02),
    ("2", 0x03),
    ("3", 0x04),
    ("4", 0x05),
    ("5", 0x06),
    ("6", 0x07),
    ("7", 0x08),
    ("8", 0x09),
    ("9", 0x0a),
    ("0", 0x0b),
    ("-", 0x0c),
    ("=", 0x0d),
    ("q", 0x10),
    ("w", 0x11),
    ("e", 0x12),
    ("r", 0x13),
    ("t", 0x14),
    ("y", 0x15),
    ("u", 0x16),
    ("i", 0x17),
    ("o", 0x18),
    ("p", 0x19),
    ("[", 0x1a),
    ("]", 0x1b),
    ("a", 0x1e),
    ("s", 0x1f),
    ("d", 0x20),
    ("f", 0x21),
    ("g", 0x22),
    ("h", 0x23),
    ("j", 0x24),
    ("k", 0x25),
    ("l", 0x26),
    (";", 0x27),
    ("'", 0x28),
    ("`", 0x29),
    ("\\", 0x2b),
    ("z", 0x2c),
    ("x", 0x2d),
    ("c", 0x2e),
    ("v", 0x2f),
    ("b", 0x30),
    ("n", 0x31),
    ("m", 0x32),
    (",", 0x33),
    (".", 0x34),
    ("/", 0x35),
    ("intlbackslash", 0x56),
];

/// What the recorder saves for the physical keys of the active layout.
#[derive(Serialize, Type, Debug, Default)]
pub struct RecordedKeys {
    /// The name of the virtual key under each physical key whose name
    /// differs, keyed by the physical key's name.
    pub names: HashMap<String, String>,
    /// The physical keys whose virtual key no shortcut name registers, such
    /// as the "!" key on AZERTY: saved under their own name, a shortcut would
    /// fire on another key.
    pub unusable: Vec<String>,
}

/// The label of each layout key, from `typed`: what the active layout types
/// on a virtual key code. A key it types nothing printable on keeps its name.
pub fn labels(typed: impl Fn(u16) -> Option<char>) -> HashMap<String, String> {
    LAYOUT_KEYS
        .iter()
        .filter_map(|&(name, code)| {
            typed(code)
                .filter(|c| !c.is_control() && !c.is_whitespace())
                .map(|c| (name.to_string(), c.to_string()))
        })
        .collect()
}

/// What the recorder saves for each physical key, from `virtual_key`: the
/// layout's virtual key on a scan code. On AZERTY the key a US keyboard calls
/// "q" is the A key, and no shortcut name registers the "!" key's virtual key.
/// A scan code the layout puts no virtual key on keeps its own name.
#[cfg(target_os = "windows")]
pub fn names(virtual_key: impl Fn(u16) -> Option<u16>) -> RecordedKeys {
    let mut keys = RecordedKeys::default();
    for &(physical, scan_code) in &PHYSICAL_KEYS {
        let Some(key) = virtual_key(scan_code) else {
            continue;
        };
        match LAYOUT_KEYS.iter().find(|&&(_, code)| code == key) {
            Some(&(name, _)) if name != physical => {
                keys.names.insert(physical.to_string(), name.to_string());
            }
            Some(_) => {}
            None => keys.unusable.push(physical.to_string()),
        }
    }
    keys
}

/// The label of each layout key on the active layout. Call it on the main
/// thread: macOS answers only there.
#[cfg(target_os = "macos")]
pub fn read() -> Result<HashMap<String, String>, String> {
    let chars = crate::input::command_key_chars()?;
    Ok(labels(|code| {
        chars.get(usize::from(code)).copied().flatten()
    }))
}

/// macOS registers the physical key, so every recorded key keeps its name.
#[cfg(target_os = "macos")]
pub fn read_names() -> Result<RecordedKeys, String> {
    Ok(RecordedKeys::default())
}

/// The label of each layout key on the active layout. Call it on the main
/// thread: the layout belongs to a thread, and that one owns Murmur's windows.
#[cfg(target_os = "windows")]
pub fn read() -> Result<HashMap<String, String>, String> {
    active_layout().map(labels_on)
}

/// What the recorder must store for each physical key on the active layout,
/// so that a shortcut fires on the key the user pressed. Call it on the main
/// thread, as [`read`].
#[cfg(target_os = "windows")]
pub fn read_names() -> Result<RecordedKeys, String> {
    active_layout().map(names_on)
}

#[cfg(target_os = "windows")]
fn active_layout() -> Result<HKL, String> {
    // SAFETY: plain query of the calling thread's keyboard layout.
    let layout = unsafe { GetKeyboardLayout(0) };
    if layout.0.is_null() {
        Err("Windows returned no keyboard layout".into())
    } else {
        Ok(layout)
    }
}

#[cfg(target_os = "windows")]
fn labels_on(layout: HKL) -> HashMap<String, String> {
    labels(|virtual_key| {
        // SAFETY: plain lookup in a loaded keyboard layout. Unlike
        // `ToUnicodeEx`, it leaves the thread's dead-key state alone.
        let typed =
            unsafe { MapVirtualKeyExW(u32::from(virtual_key), MAPVK_VK_TO_CHAR, Some(layout)) };
        // The top bit marks a dead key, labelled with its accent as macOS
        // does. Letters come back in upper case.
        char::from_u32(typed & 0xffff).and_then(|c| c.to_lowercase().next())
    })
}

#[cfg(target_os = "windows")]
fn names_on(layout: HKL) -> RecordedKeys {
    names(|scan_code| {
        // SAFETY: plain lookup in a loaded keyboard layout.
        let key = unsafe { MapVirtualKeyExW(u32::from(scan_code), MAPVK_VSC_TO_VK, Some(layout)) };
        u16::try_from(key).ok().filter(|&key| key != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layout_key_is_a_key_a_shortcut_can_name() {
        for (name, _) in LAYOUT_KEYS {
            let shortcut = format!("ctrl+{name}");
            assert!(
                crate::shortcut::tauri_impl::parse_shortcut(&shortcut).is_ok(),
                "{shortcut} does not parse"
            );
        }
    }

    /// A few keys of the French AZERTY layout, by virtual key code.
    #[cfg(target_os = "macos")]
    fn azerty(code: u16) -> Option<char> {
        match code {
            0x00 => Some('q'),
            0x0c => Some('a'),
            0x29 => Some('m'),
            0x2e => Some(','),
            0x09 => Some('v'),
            0x32 => Some('\u{10}'),
            _ => None,
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_key_is_labelled_with_what_the_layout_types_on_it() {
        let labels = labels(azerty);
        // The default meeting shortcut's key types "," on AZERTY, and the
        // key printed M is the one the parser calls ";".
        assert_eq!(labels["m"], ",");
        assert_eq!(labels[";"], "m");
        assert_eq!(labels["a"], "q");
        assert_eq!(labels["q"], "a");
        assert_eq!(labels["v"], "v");
        assert!(!labels.contains_key("`"));
        assert!(!labels.contains_key("k"));
    }

    /// A few keys of the French AZERTY layout, by virtual key: what each one
    /// types, as `MapVirtualKeyExW` answers.
    #[cfg(target_os = "windows")]
    fn azerty_typed(key: u16) -> Option<char> {
        match key {
            key if key == VK_1.0 => Some('&'),
            key if key == VK_A.0 => Some('a'),
            key if key == VK_OEM_COMMA.0 => Some(','),
            key if key == VK_OEM_2.0 => Some(':'),
            key if key == VK_OEM_7.0 => Some('²'),
            key if key == VK_OEM_PLUS.0 => Some('\u{10}'),
            _ => None,
        }
    }

    /// The same layout by scan code: the virtual key under each physical key.
    #[cfg(target_os = "windows")]
    fn azerty_keys(scan_code: u16) -> Option<u16> {
        match scan_code {
            0x02 => Some(VK_1.0),
            0x10 => Some(VK_A.0),
            0x1e => Some(VK_Q.0),
            0x27 => Some(VK_M.0),
            0x32 => Some(VK_OEM_COMMA.0),
            0x34 => Some(VK_OEM_2.0),
            // The "!" and "<" keys, which no shortcut name reaches.
            0x35 => Some(VK_OEM_8.0),
            0x56 => Some(VK_OEM_102.0),
            _ => None,
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_virtual_key_is_labelled_with_what_the_layout_types_on_it() {
        let labels = labels(azerty_typed);
        assert_eq!(labels["1"], "&");
        assert_eq!(labels["a"], "a");
        assert_eq!(labels[","], ",");
        assert_eq!(labels["/"], ":");
        assert_eq!(labels["'"], "²");
        assert!(!labels.contains_key("="));
        assert!(!labels.contains_key("k"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_physical_key_is_recorded_as_the_virtual_key_on_it() {
        let names = names(azerty_keys).names;
        assert_eq!(names["q"], "a");
        assert_eq!(names["a"], "q");
        assert_eq!(names[";"], "m");
        assert_eq!(names["m"], ",");
        assert_eq!(names["."], "/");
        // The digits keep their names: "ctrl+1" is the key printed "&".
        assert!(!names.contains_key("1"));
        assert!(!names.contains_key("/"));
        assert!(!names.contains_key("k"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_physical_key_no_shortcut_name_registers_is_unusable() {
        // A key the layout puts no virtual key on ("k" here) is not refused.
        assert_eq!(names(azerty_keys).unusable, ["/", "intlbackslash"]);
    }

    /// The layouts this session has loaded.
    #[cfg(target_os = "windows")]
    fn loaded_layouts() -> Vec<HKL> {
        // SAFETY: the first call only counts, the second fills `layouts`.
        let count = unsafe { GetKeyboardLayoutList(None) };
        let mut layouts = vec![HKL(std::ptr::null_mut()); usize::try_from(count).unwrap_or(0)];
        let filled = unsafe { GetKeyboardLayoutList(Some(&mut layouts)) };
        layouts.truncate(usize::try_from(filled).unwrap_or(0));
        layouts
    }

    /// The labels and recorded keys of a layout that ships with Windows, by
    /// its layout id.
    #[cfg(target_os = "windows")]
    fn read_layout(id: windows::core::PCWSTR) -> (HashMap<String, String>, RecordedKeys) {
        let loaded = loaded_layouts();
        // SAFETY: the shell is not told, and the layout is unloaded below
        // unless the user already had it.
        let layout = unsafe { LoadKeyboardLayoutW(id, KLF_NOTELLSHELL) }.expect("the layout loads");
        let read = (labels_on(layout), names_on(layout));
        if !loaded.contains(&layout) {
            // SAFETY: `layout` was loaded above and is not active anywhere.
            let _ = unsafe { UnloadKeyboardLayout(layout) };
        }
        read
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_french_layout_labels_and_names_its_keys() {
        let (labels, keys) = read_layout(windows::core::w!("0000040C"));

        assert_eq!(labels["1"], "&");
        assert_eq!(labels["a"], "a");
        assert_eq!(labels[","], ",");
        assert_eq!(labels["/"], ":");
        // A dead key shows its accent.
        assert_eq!(labels["]"], "^");
        let names = keys.names;
        assert_eq!(names["q"], "a");
        assert_eq!(names["a"], "q");
        assert_eq!(names[";"], "m");
        assert_eq!(names["m"], ",");
        assert!(!names.contains_key("1"));
        assert!(!names.contains_key("/"));
        // The "!" key and the "<" key.
        assert_eq!(keys.unusable, ["/", "intlbackslash"]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_uk_layout_refuses_its_backquote_key() {
        let (_, keys) = read_layout(windows::core::w!("00000809"));

        // The key left of 1 types "`" there too, on a virtual key no shortcut
        // name registers.
        assert_eq!(keys.unusable, ["`"]);
        // The "\" key between left Shift and Z registers as "\".
        assert_eq!(keys.names["intlbackslash"], "\\");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_us_layout_records_every_key_under_its_name() {
        let (_, keys) = read_layout(windows::core::w!("00000409"));

        assert!(keys.names.is_empty(), "{:?}", keys.names);
        // The ISO key a US keyboard lacks.
        assert_eq!(keys.unusable, ["intlbackslash"]);
    }
}
