import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import PhosphorIconProvider from "./components/icons/PhosphorIconProvider";
import { installCompatShims } from "./lib/compat";
import {
  applyTheme,
  getStoredTheme,
  syncThemeFromSettings,
} from "./lib/utils/theme";
import {
  applyAccentColor,
  getStoredAccentColor,
  syncAccentColorFromSettings,
} from "./lib/utils/accentColor";

installCompatShims();

// Apply the last-known theme synchronously before render to avoid a flash of
// the wrong palette, then reconcile with the persisted setting once it loads.
applyTheme(getStoredTheme());
syncThemeFromSettings();
applyAccentColor(getStoredAccentColor());
syncAccentColorFromSettings();

// Initialize i18n
import "./i18n";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <PhosphorIconProvider>
      <App />
    </PhosphorIconProvider>
  </React.StrictMode>,
);
