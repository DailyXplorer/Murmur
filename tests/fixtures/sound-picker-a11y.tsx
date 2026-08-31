import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { I18nextProvider, initReactI18next } from "react-i18next";
import type { AppSettings } from "../../src/bindings";
import { SoundPicker } from "../../src/components/settings/SoundPicker";
import { useSettingsStore } from "../../src/stores/settingsStore";
import frTranslation from "../../src/i18n/locales/fr/translation.json";
import "../../src/App.css";

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
  sound_theme: "marimba",
  transcription_provider: "gemini",
};

const SoundPickerFixture: React.FC = () => (
  <main className="min-h-screen bg-background px-8 py-10 text-text">
    <section className="mx-auto flex w-[720px] flex-col gap-4">
      <I18nextProvider i18n={i18n}>
        <SoundPicker
          label={i18n.t("settings.debug.soundTheme.label")}
          description={i18n.t("settings.debug.soundTheme.description")}
        />
      </I18nextProvider>
    </section>
  </main>
);

const renderFixture = async () => {
  await i18n.use(initReactI18next).init({
    fallbackLng: "fr",
    interpolation: { escapeValue: false },
    lng: "fr",
    react: { useSuspense: false },
    resources: { fr: { translation: frTranslation } },
  });

  useSettingsStore.setState({
    customSounds: { start: false, stop: false },
    isLoading: false,
    isUpdating: {},
    settings: TEST_SETTINGS,
  });

  const rootElement = document.getElementById("root");
  if (!rootElement) throw new Error("Fixture root is missing");

  ReactDOM.createRoot(rootElement).render(<SoundPickerFixture />);
};

void renderFixture();
