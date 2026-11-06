import { platform } from "@tauri-apps/plugin-os";

/** The Windows build offers dictation only: no meetings, updater or macOS integrations. */
export const isWindows = platform() === "windows";
