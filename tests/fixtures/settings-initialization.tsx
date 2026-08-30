import React from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings } from "../../src/bindings";
import { useSettings } from "../../src/hooks/useSettings";
import { useSettingsStore } from "../../src/stores/settingsStore";

declare global {
  interface Window {
    settingsInitialization: {
      customSounds: number;
      defaultSettings: number;
      listeners: number;
      settings: number;
      unhandledRejections: number;
      emitSettingsChanged: () => Promise<void>;
      retryListenerRegistration: () => Promise<void>;
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
  emitSettingsChanged: () => emit("settings-changed", { setting: "theme" }),
  retryListenerRegistration: () => useSettingsStore.getState().initialize(),
};

window.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  window.settingsInitialization.unhandledRejections += 1;
});

mockIPC(
  (command) => {
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
      default:
        throw new Error(`Unexpected Tauri command: ${command}`);
    }
  },
  { shouldMockEvents: true },
);

type TauriInvoke = (
  command: string,
  args?: unknown,
  options?: unknown,
) => Promise<unknown>;

const tauriInternals = window.__TAURI_INTERNALS__ as unknown as {
  invoke: TauriInvoke;
};
const invoke = tauriInternals.invoke;
let remainingListenerFailures = listenerShouldFail ? 1 : 0;

tauriInternals.invoke = async (command, args, options) => {
  if (command === "plugin:event|listen") {
    window.settingsInitialization.listeners += 1;

    if (remainingListenerFailures > 0) {
      remainingListenerFailures -= 1;
      throw new Error("Expected listener registration failure");
    }
  }

  return invoke(command, args, options);
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <SettingsConsumer />
    <SettingsConsumer />
    <SettingsConsumer />
  </React.StrictMode>,
);
