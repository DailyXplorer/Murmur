import { isWindows } from "@/lib/platform";

export const getKeyName = (e: KeyboardEvent): string => {
  if (e.code) {
    const code = e.code;

    if (code.match(/^F\d+$/)) {
      return code.toLowerCase();
    }

    if (code.match(/^Key[A-Z]$/)) {
      return code.replace("Key", "").toLowerCase();
    }

    if (code.match(/^Digit\d$/)) {
      return code.replace("Digit", "");
    }

    const modifierMap: Record<string, string> = {
      ShiftLeft: "shift",
      ShiftRight: "shift",
      ControlLeft: "ctrl",
      ControlRight: "ctrl",
      AltLeft: "option",
      AltRight: "option",
      MetaLeft: "command",
      MetaRight: "command",
      Tab: "tab",
      Enter: "enter",
      Space: "space",
      Backspace: "backspace",
      Delete: "delete",
      Escape: "esc",
      ArrowUp: "up",
      ArrowDown: "down",
      ArrowLeft: "left",
      ArrowRight: "right",
      Home: "home",
      End: "end",
      Insert: "insert",
      Pause: "pause",
      ContextMenu: "menu",
    };

    if (modifierMap[code]) {
      return modifierMap[code];
    }

    const punctuationMap: Record<string, string> = {
      Semicolon: ";",
      Equal: "=",
      Comma: ",",
      Minus: "-",
      Period: ".",
      Slash: "/",
      Backquote: "`",
      BracketLeft: "[",
      Backslash: "\\",
      BracketRight: "]",
      Quote: "'",
    };

    if (punctuationMap[code]) {
      return punctuationMap[code];
    }

    // The remaining codes (PageUp, CapsLock, Numpad1, NumpadAdd, ...) are
    // spelled exactly as the global-hotkey parser names them.
    return code.toLowerCase();
  }

  if (e.key) {
    const key = e.key;

    const keyMap: Record<string, string> = {
      Control: "ctrl",
      Alt: "option",
      Shift: "shift",
      Meta: "command",
      CapsLock: "capslock",
      ArrowUp: "up",
      ArrowDown: "down",
      ArrowLeft: "left",
      ArrowRight: "right",
      Escape: "esc",
      " ": "space",
    };

    if (keyMap[key]) {
      return keyMap[key];
    }

    return key.toLowerCase();
  }

  return `unknown-${e.keyCode || e.which || 0}`;
};

const KEY_LABELS: Record<string, string> = {
  pageup: "Page Up",
  pagedown: "Page Down",
  capslock: "Caps Lock",
  printscreen: "Print Screen",
  scrolllock: "Scroll Lock",
  numlock: "Num Lock",
  numpadadd: "Numpad +",
  numpadsubtract: "Numpad -",
  numpadmultiply: "Numpad *",
  numpaddivide: "Numpad /",
  numpaddecimal: "Numpad .",
  numpadenter: "Numpad Enter",
  numpadequal: "Numpad =",
};

// Stored tokens keep their macOS names; Windows reads Option as Alt and
// Command as the Windows key.
const WINDOWS_MODIFIER_LABELS: Record<string, string> = {
  ctrl: "Ctrl",
  control: "Ctrl",
  option: "Alt",
  alt: "Alt",
  command: "Win",
  cmd: "Win",
  super: "Win",
  meta: "Win",
};

const capitalizeKey = (key: string): string => {
  if (isWindows && WINDOWS_MODIFIER_LABELS[key]) {
    return WINDOWS_MODIFIER_LABELS[key];
  }
  if (KEY_LABELS[key]) return KEY_LABELS[key];
  const numpadDigit = /^numpad(\d)$/.exec(key);
  if (numpadDigit) return `Numpad ${numpadDigit[1]}`;
  if (key === "fn") return "fn";
  if (/^f\d+$/.test(key)) return key.toUpperCase();
  if (key.length === 1) return key.toUpperCase();
  return key.replace(/\b\w/g, (c) => c.toUpperCase());
};

/**
 * What the active keyboard layout writes on a key, by the key's name in a
 * shortcut: on AZERTY, "m" is the "," key.
 */
export type KeyLabels = Partial<Record<string, string>>;

const formatKeyPart = (part: string, labels: KeyLabels): string => {
  const trimmed = part.trim();
  if (!trimmed) return "";
  const label = labels[trimmed];
  if (label) return capitalizeKey(label);

  if (trimmed.endsWith("_left")) {
    const name = trimmed.slice(0, -5);
    return `Left ${capitalizeKey(name)}`;
  }
  if (trimmed.endsWith("_right")) {
    const name = trimmed.slice(0, -6);
    return `Right ${capitalizeKey(name)}`;
  }

  return capitalizeKey(trimmed);
};

export const formatKeyCombination = (
  combination: string,
  labels: KeyLabels = {},
): string => {
  if (!combination) return "";
  return combination
    .split("+")
    .map((part) => formatKeyPart(part, labels))
    .join(" + ");
};

export const normalizeKey = (key: string): string => {
  if (key.startsWith("left ") || key.startsWith("right ")) {
    const parts = key.split(" ");
    if (parts.length === 2) {
      return parts[1];
    }
  }
  return key;
};
