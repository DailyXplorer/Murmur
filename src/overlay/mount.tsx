import React from "react";
import ReactDOM from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import {
  applyTheme,
  getStoredTheme,
  syncThemeFromSettings,
} from "@/lib/utils/theme";
import type { Theme } from "@/bindings";

/** Renders `overlay` in its own webview. Each sets `data-theme` on its own
 * document: last-known theme before render (shared localStorage) to avoid
 * a flash, reconciled with the persisted setting in case it booted first,
 * then following live changes. */
export function mountOverlay(overlay: React.ReactNode) {
  applyTheme(getStoredTheme());
  syncThemeFromSettings();
  listen<Theme>("theme-changed", (event) => applyTheme(event.payload));

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>{overlay}</React.StrictMode>,
  );
}
