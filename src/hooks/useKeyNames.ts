import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import type { KeyNames } from "../lib/utils/keyboard";

/** The keys a shortcut names with a word of the language: the space bar
 * has no label to read. Modifiers and letters keep what the keyboard
 * prints. */
const NAMED_KEYS = new Set([
  "space",
  "enter",
  "escape",
  "esc",
  "tab",
  "backspace",
  "delete",
  "up",
  "down",
  "left",
  "right",
]);

/** Names the named keys of a shortcut in the app's language. */
export const useKeyNames = (): KeyNames => {
  const { t } = useTranslation();
  return useCallback(
    (key) => (NAMED_KEYS.has(key) ? t(`keyNames.${key}`) : undefined),
    [t],
  );
};
