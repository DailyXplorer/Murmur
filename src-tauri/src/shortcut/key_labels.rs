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

/// The same keys as physical keys: the name the recorder gives each one (what
/// it types on a US keyboard), with its scan code.
#[cfg(target_os = "windows")]
const PHYSICAL_KEYS: [(&str, u16); 47] = [
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
];

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

/// The name of the virtual key under each physical key whose name differs,
/// from `virtual_key`: the layout's virtual key on a scan code. On AZERTY the
/// key a US keyboard calls "q" is the A key. A physical key whose virtual key
/// no shortcut can name is left out, and keeps its own name.
#[cfg(target_os = "windows")]
pub fn names(virtual_key: impl Fn(u16) -> Option<u16>) -> HashMap<String, String> {
    PHYSICAL_KEYS
        .iter()
        .filter_map(|&(physical, scan_code)| {
            let key = virtual_key(scan_code)?;
            let &(name, _) = LAYOUT_KEYS.iter().find(|&&(_, code)| code == key)?;
            (name != physical).then(|| (physical.to_string(), name.to_string()))
        })
        .collect()
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
pub fn read_names() -> Result<HashMap<String, String>, String> {
    Ok(HashMap::new())
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
pub fn read_names() -> Result<HashMap<String, String>, String> {
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
fn names_on(layout: HKL) -> HashMap<String, String> {
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
            // The "!" key, which no shortcut name reaches.
            0x35 => Some(VK_OEM_8.0),
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
        let names = names(azerty_keys);
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

    #[cfg(target_os = "windows")]
    #[test]
    fn the_french_layout_labels_and_names_its_keys() {
        let loaded = loaded_layouts();
        // SAFETY: kbdfr.dll ships with Windows; the shell is not told, and
        // the layout is unloaded below unless the user already had it.
        let french = unsafe { LoadKeyboardLayoutW(windows::core::w!("0000040C"), KLF_NOTELLSHELL) }
            .expect("the French layout loads");
        let labels = labels_on(french);
        let names = names_on(french);
        if !loaded.contains(&french) {
            // SAFETY: `french` was loaded above and is not active anywhere.
            let _ = unsafe { UnloadKeyboardLayout(french) };
        }

        assert_eq!(labels["1"], "&");
        assert_eq!(labels["a"], "a");
        assert_eq!(labels[","], ",");
        assert_eq!(labels["/"], ":");
        // A dead key shows its accent.
        assert_eq!(labels["]"], "^");
        assert_eq!(names["q"], "a");
        assert_eq!(names["a"], "q");
        assert_eq!(names[";"], "m");
        assert_eq!(names["m"], ",");
        assert!(!names.contains_key("1"));
        assert!(!names.contains_key("/"));
    }
}
