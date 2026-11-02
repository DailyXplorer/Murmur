import React from "react";
import { GlobalShortcutInput } from "./GlobalShortcutInput";

interface ShortcutInputProps {
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
}

export const ShortcutInput: React.FC<ShortcutInputProps> = (props) => {
  return <GlobalShortcutInput {...props} />;
};
