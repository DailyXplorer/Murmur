import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AppSettings,
  AudioDevice,
  ShortcutBinding,
} from "../../src/bindings";
import { CustomWords } from "../../src/components/settings/CustomWords";
import { GlobalShortcutInput } from "../../src/components/settings/GlobalShortcutInput";
import { HistoryLimit } from "../../src/components/settings/HistoryLimit";
import { LanguageSelector } from "../../src/components/settings/LanguageSelector";
import { MicrophoneSelector } from "../../src/components/settings/MicrophoneSelector";
import { TranscriptionSettings } from "../../src/components/settings/transcription/TranscriptionSettings";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

export const LONG_CUSTOM_WORD = "W".repeat(50);

const DEFAULT_DEVICE: AudioDevice = {
  index: "default",
  is_default: true,
  name: "Default",
};

const TRANSCRIBE_BINDING: ShortcutBinding = {
  current_binding: "Alt+Space",
  default_binding: "Alt+Space",
  description: "Record and transcribe",
  id: "transcribe",
  name: "Transcribe Shortcut",
};

const TEST_SETTINGS: AppSettings = {
  append_trailing_space: false,
  auto_submit: false,
  auto_submit_key: "enter",
  bindings: { transcribe: TRANSCRIBE_BINDING },
  clipboard_handling: "dont_modify",
  custom_words: ["Acme", LONG_CUSTOM_WORD],
  filler_word_removal_enabled: false,
  history_limit: 50,
  paste_method: "ctrl_v",
  selected_language: "auto",
  selected_microphone: "Default",
  transcription_provider: "gemini",
};

const ProductionSettingsControlWidthsFixture: React.FC = () => (
  <main className="min-h-screen bg-background px-8 py-10 text-text">
    <section className="mx-auto flex w-[720px] flex-col gap-4">
      <div data-testid="production-microphone">
        <MicrophoneSelector />
      </div>
      <div data-testid="production-language">
        <LanguageSelector />
      </div>
      <div data-testid="production-shortcut">
        <GlobalShortcutInput shortcutId="transcribe" />
      </div>
      <div data-testid="production-history-limit">
        <HistoryLimit />
      </div>
      <div data-testid="production-custom-words">
        <CustomWords />
      </div>
      <div data-testid="production-transcription">
        <TranscriptionSettings />
      </div>
    </section>
  </main>
);

const renderFixture = async () => {
  let metaConfigured = false;
  mockIPC((command) => {
    if (command === "get_codex_auth_status") return { signed_in: true };
    if (command === "get_gemini_status") {
      return { installed: true, signed_in: true };
    }
    if (command === "get_meta_api_status") {
      return { configured: metaConfigured };
    }
    if (command === "save_meta_api_key") {
      metaConfigured = true;
      return null;
    }
    if (command === "clear_meta_api_key") {
      metaConfigured = false;
      return null;
    }
    throw new Error(`Unexpected Tauri command: ${command}`);
  });

  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: "en",
    react: { useSuspense: false },
    resources: { en: { translation: enTranslation } },
  });

  useSettingsStore.setState({
    audioDevices: [DEFAULT_DEVICE],
    defaultSettings: TEST_SETTINGS,
    isLoading: false,
    isUpdating: {},
    outputDevices: [DEFAULT_DEVICE],
    settings: TEST_SETTINGS,
  });

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <ProductionSettingsControlWidthsFixture />,
  );
};

void renderFixture();
