import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import type { AppSettings, AudioDevice } from "../../src/bindings";
import { LanguageSelector } from "../../src/components/settings/LanguageSelector";
import { MicrophoneSelector } from "../../src/components/settings/MicrophoneSelector";
import { OutputDeviceSelector } from "../../src/components/settings/OutputDeviceSelector";
import { RecordingRetentionPeriodSelector } from "../../src/components/settings/RecordingRetentionPeriod";
import { useSettingsStore } from "../../src/stores/settingsStore";
import frTranslation from "../../src/i18n/locales/fr/translation.json";
import "../../src/App.css";

const DEFAULT_DEVICE: AudioDevice = {
  index: "default",
  is_default: true,
  name: "Default",
};

const TEST_SETTINGS: AppSettings = {
  append_trailing_space: false,
  auto_submit: false,
  auto_submit_key: "enter",
  bindings: {},
  clipboard_handling: "dont_modify",
  custom_words: [],
  ai_cleanup_enabled: false,
  history_limit: 50,
  paste_method: "ctrl_v",
  selected_language: "auto",
  selected_microphone: "Default",
  selected_output_device: "Default",
};

const SettingsFrenchLabelsFixture: React.FC = () => (
  <main className="min-h-screen bg-background px-8 py-10 text-text">
    <section className="mx-auto flex w-[720px] flex-col gap-4">
      <div data-testid="microphone">
        <MicrophoneSelector />
      </div>
      <div data-testid="output-device">
        <OutputDeviceSelector />
      </div>
      <div data-testid="language">
        <LanguageSelector />
      </div>
      <div data-testid="retention">
        <RecordingRetentionPeriodSelector descriptionMode="inline" />
      </div>
    </section>
  </main>
);

const renderFixture = async () => {
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: "fr",
    react: { useSuspense: false },
    resources: { fr: { translation: frTranslation } },
  });

  useSettingsStore.setState({
    audioDevices: [DEFAULT_DEVICE],
    defaultSettings: TEST_SETTINGS,
    isLoading: false,
    isUpdating: {},
    outputDevices: [DEFAULT_DEVICE],
    settings: TEST_SETTINGS,
  });

  const rootElement = document.getElementById("root");
  if (!rootElement) throw new Error("Fixture root is missing");

  ReactDOM.createRoot(rootElement).render(<SettingsFrenchLabelsFixture />);
};

void renderFixture();
