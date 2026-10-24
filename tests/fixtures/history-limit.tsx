import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings } from "../../src/bindings";
import { HistoryLimit } from "../../src/components/settings/HistoryLimit";
import { RecordingRetentionPeriodSelector } from "../../src/components/settings/RecordingRetentionPeriod";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import frTranslation from "../../src/i18n/locales/fr/translation.json";
import "../../src/App.css";

const params = new URLSearchParams(window.location.search);
// `?lang=fr&keep=1` keeps the latest dictation only.
const language = params.get("lang") ?? "en";
const kept = params.get("keep");

declare global {
  interface Window {
    historyLimit: {
      limitWrites: () => number[];
    };
  }
}

const limitWrites: number[] = [];

mockIPC((command, args) => {
  if (command === "update_history_limit") {
    limitWrites.push((args as { limit: number }).limit);
    return null;
  }
  throw new Error(`Unexpected Tauri command: ${command}`);
});

const renderFixture = async () => {
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: language,
    react: { useSuspense: false },
    resources: {
      en: { translation: enTranslation },
      fr: { translation: frTranslation },
    },
  });

  const settings: AppSettings = kept
    ? {
        history_limit: Number(kept),
        recording_retention_period: "preserve_limit",
      }
    : { history_limit: 50 };
  useSettingsStore.setState({
    isLoading: false,
    isUpdating: {},
    settings,
  });

  const rootElement = document.getElementById("root");
  if (!rootElement)
    throw new Error("History limit fixture is missing its root");
  window.historyLimit = { limitWrites: () => [...limitWrites] };
  ReactDOM.createRoot(rootElement).render(
    <main className="min-h-screen bg-background px-8 py-10 text-text">
      <HistoryLimit />
      <RecordingRetentionPeriodSelector descriptionMode="inline" />
      <button type="button">Elsewhere</button>
    </main>,
  );
};

void renderFixture();
