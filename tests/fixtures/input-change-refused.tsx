import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import { Toaster } from "sonner";
import type { AppSettings, AudioDevice } from "../../src/bindings";
import { ChannelSelector } from "../../src/components/settings/ChannelSelector";
import { MicrophoneSelector } from "../../src/components/settings/MicrophoneSelector";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

const SETTINGS = {
  selected_microphone: "Built-in Microphone",
  selected_channel: null,
} as AppSettings;

const AUDIO_DEVICES: AudioDevice[] = [
  { index: "default", is_default: true, name: "Default" },
  { index: "built-in", is_default: false, name: "Built-in Microphone" },
  { index: "usb", is_default: false, name: "USB Microphone" },
];

// A dictation is running: the backend refuses to switch the input.
mockIPC((command) => {
  switch (command) {
    case "get_available_microphones":
      return AUDIO_DEVICES;
    case "get_microphone_channels":
      return 2;
    case "set_selected_microphone":
      return Promise.reject("Cannot change the microphone while recording");
    case "set_selected_channel":
      return Promise.reject(
        "Failed to update channel selection: Cannot change the input channel while recording",
      );
    default:
      throw new Error(`Unexpected Tauri command: ${command}`);
  }
});

const render = async () => {
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: "en",
    react: { useSuspense: false },
    resources: { en: { translation: enTranslation } },
  });
  useSettingsStore.setState({
    audioDevices: AUDIO_DEVICES,
    defaultSettings: { ...SETTINGS, selected_microphone: "default" },
    isLoading: false,
    isUpdating: {},
    settings: { ...SETTINGS },
  });
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <main className="flex flex-col gap-4 p-8">
      <section data-testid="microphone">
        <MicrophoneSelector />
      </section>
      <section data-testid="channel">
        <ChannelSelector />
      </section>
      <Toaster />
    </main>,
  );
};

void render();
