//! What the active keyboard layout writes on the keys a shortcut names. A
//! shortcut is registered on a physical key, named after what that key types
//! on a US keyboard: "ctrl+option+command+m" is the key left of "," there,
//! which is the "," key on a French AZERTY keyboard. Its label must say ",".

use std::collections::HashMap;

/// The keys whose character moves with the layout, by the name the shortcut
/// parser gives them, with the virtual key code global-hotkey registers.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A few keys of the French AZERTY layout, by virtual key code.
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
}
