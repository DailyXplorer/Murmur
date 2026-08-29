import { commands, type AccentColor } from "@/bindings";
import { listen } from "@tauri-apps/api/event";

const ACCENT_COLOR_STORAGE_KEY = "murmur.accent-color";

export const ACCENT_COLOR_OPTIONS: AccentColor[] = [
  "pink",
  "blue",
  "green",
  "yellow",
  "orange",
  "red",
];

export const ACCENT_COLOR_PREVIEWS: Record<AccentColor, string> = {
  pink: "#faa2ca",
  blue: "#9cc7ff",
  green: "#99e8b7",
  yellow: "#f2d66b",
  orange: "#f9bc82",
  red: "#f6a0a3",
};

const isAccentColor = (value: unknown): value is AccentColor =>
  typeof value === "string" &&
  ACCENT_COLOR_OPTIONS.includes(value as AccentColor);

export const applyAccentColor = (accentColor: AccentColor): void => {
  document.documentElement.dataset.accentColor = accentColor;
  try {
    localStorage.setItem(ACCENT_COLOR_STORAGE_KEY, accentColor);
  } catch {
    // AppSettings remains the source of truth when localStorage is unavailable.
  }
};

export const getStoredAccentColor = (): AccentColor => {
  try {
    const stored = localStorage.getItem(ACCENT_COLOR_STORAGE_KEY);
    if (isAccentColor(stored)) return stored;
  } catch {
    // Ignore localStorage failures and keep the original pink accent.
  }
  return "pink";
};

export const syncAccentColorFromSettings = async (): Promise<void> => {
  let eventRevision = 0;

  try {
    await listen<AccentColor>("accent-color-changed", (event) => {
      eventRevision += 1;
      applyAccentColor(event.payload);
    });
  } catch (error) {
    console.warn("Failed to listen for accent color changes:", error);
  }

  try {
    const requestRevision = eventRevision;
    const result = await commands.getAppSettings();
    if (result.status === "ok" && eventRevision === requestRevision) {
      applyAccentColor(result.data.accent_color ?? "pink");
    }
  } catch (error) {
    console.warn("Failed to sync accent color from settings:", error);
  }
};
