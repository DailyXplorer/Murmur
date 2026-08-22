import React from "react";
import { GlobalShortcutInput } from "./GlobalShortcutInput";

interface ShortcutInputProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
}

export const ShortcutInput: React.FC<ShortcutInputProps> = (props) => {
  return <GlobalShortcutInput {...props} />;
};
