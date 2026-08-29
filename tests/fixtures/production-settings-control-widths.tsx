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
import { AppDataDirectory } from "../../src/components/settings/AppDataDirectory";
import { CustomWords } from "../../src/components/settings/CustomWords";
import { GlobalShortcutInput } from "../../src/components/settings/GlobalShortcutInput";
import { HistoryLimit } from "../../src/components/settings/HistoryLimit";
import { LanguageSelector } from "../../src/components/settings/LanguageSelector";
import { MicrophoneSelector } from "../../src/components/settings/MicrophoneSelector";
import { PushToTalk } from "../../src/components/settings/PushToTalk";
import { LogDirectory } from "../../src/components/settings/debug/LogDirectory";
import { TranscriptionSettings } from "../../src/components/settings/transcription/TranscriptionSettings";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

export const LONG_CUSTOM_WORD = "W".repeat(50);
export const APP_DATA_PATH =
  "/Users/example/Library/Application Support/com.dailyxplorer.murmur";
export const LOG_DIRECTORY_PATH =
  "/Users/example/Library/Logs/com.dailyxplorer.murmur";

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

const CANCEL_BINDING: ShortcutBinding = {
  current_binding: "Escape",
  default_binding: "Escape",
  description: "Cancel transcription",
  id: "cancel",
  name: "Cancel Shortcut",
};

const TEST_SETTINGS: AppSettings = {
  append_trailing_space: false,
  auto_submit: false,
  auto_submit_key: "enter",
  bindings: {
    cancel: CANCEL_BINDING,
    transcribe: TRANSCRIBE_BINDING,
  },
  clipboard_handling: "dont_modify",
  custom_words: ["Acme", LONG_CUSTOM_WORD],
  filler_word_removal_enabled: false,
  history_limit: 50,
  paste_method: "ctrl_v",
  push_to_talk: false,
  selected_language: "auto",
  selected_microphone: "Default",
  transcription_provider: "gemini",
};

const ProductionSettingsControlWidthsFixture: React.FC = () => (
  <main className="min-h-screen bg-background px-4 py-10 text-text">
    <section className="mx-auto flex w-full max-w-[720px] flex-col gap-4">
      <div data-testid="production-microphone">
        <MicrophoneSelector />
      </div>
      <div data-testid="production-language">
        <LanguageSelector />
      </div>
      <div data-testid="production-transcribe-shortcut">
        <GlobalShortcutInput shortcutId="transcribe" />
      </div>
      <div data-testid="production-cancel-shortcut">
        <GlobalShortcutInput shortcutId="cancel" />
      </div>
      <div data-testid="production-push-to-talk">
        <PushToTalk />
      </div>
      <div data-testid="production-history-limit">
        <HistoryLimit />
      </div>
      <div data-testid="production-custom-words">
        <CustomWords />
      </div>
      <div data-testid="production-app-data">
        <AppDataDirectory />
      </div>
      <div data-testid="production-log-directory">
        <LogDirectory />
      </div>
      <div data-testid="production-transcription">
        <TranscriptionSettings />
      </div>
    </section>
  </main>
);

const renderFixture = async () => {
  mockIPC((command) => {
    if (command === "get_codex_auth_status") return { signed_in: true };
    if (command === "get_gemini_status") {
      return { installed: true, signed_in: true };
    }
    if (command === "get_app_dir_path") return APP_DATA_PATH;
    if (command === "get_log_dir_path") return LOG_DIRECTORY_PATH;
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
