import { platform } from "@tauri-apps/plugin-os";

/** The Windows build has no macOS integrations (Accessibility, the menu bar, Apple silicon models). */
export const isWindows = platform() === "windows";
