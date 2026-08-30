import React from "react";
import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings } from "../../src/bindings";
import { useSettings } from "../../src/hooks/useSettings";

declare global {
  interface Window {
    settingsInitialization: {
      customSounds: number;
      defaultSettings: number;
      listeners: number;
      settings: number;
      unhandledRejections: number;
    };
  }
}

const TEST_SETTINGS: AppSettings = {
  append_trailing_space: false,
  auto_submit: false,
  auto_submit_key: "enter",
  bindings: {},
  clipboard_handling: "dont_modify",
  custom_words: [],
  filler_word_removal_enabled: false,
  history_limit: 50,
  paste_method: "ctrl_v",
  selected_language: "auto",
  selected_microphone: "Default",
  transcription_provider: "gemini",
};

const respondAfterDelay = <Value,>(value: Value): Promise<Value> =>
  new Promise((resolve) => {
    window.setTimeout(() => resolve(value), 25);
  });

const SettingsConsumer: React.FC = () => {
  const { isLoading } = useSettings();

  return <output>{isLoading ? "loading" : "ready"}</output>;
};

const listenerShouldFail = new URLSearchParams(window.location.search).has(
  "listener-error",
);

window.settingsInitialization = {
  customSounds: 0,
  defaultSettings: 0,
  listeners: 0,
  settings: 0,
  unhandledRejections: 0,
};

window.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  window.settingsInitialization.unhandledRejections += 1;
});

mockIPC((command) => {
  switch (command) {
    case "get_app_settings":
      window.settingsInitialization.settings += 1;
      return respondAfterDelay(TEST_SETTINGS);
    case "get_default_settings":
      window.settingsInitialization.defaultSettings += 1;
      return respondAfterDelay(TEST_SETTINGS);
    case "check_custom_sounds":
      window.settingsInitialization.customSounds += 1;
      return respondAfterDelay({ start: false, stop: false });
    case "plugin:event|listen":
      window.settingsInitialization.listeners += 1;
      if (listenerShouldFail) {
        return Promise.reject(
          new Error("Expected listener registration failure"),
        );
      }
      return 1;
    default:
      throw new Error(`Unexpected Tauri command: ${command}`);
  }
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <SettingsConsumer />
    <SettingsConsumer />
    <SettingsConsumer />
  </React.StrictMode>,
);
