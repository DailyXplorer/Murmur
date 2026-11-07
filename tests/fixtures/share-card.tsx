import "./macos-os";
import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { ShareCard } from "../../src/components/settings/insights/share/ShareCard";
import { renderShareCard } from "../../src/components/settings/insights/share/renderShareCard";
import type { ShareStats } from "../../src/components/settings/insights/share/shareStats";
import de from "../../src/i18n/locales/de/translation.json";
import en from "../../src/i18n/locales/en/translation.json";
import es from "../../src/i18n/locales/es/translation.json";
import fr from "../../src/i18n/locales/fr/translation.json";
import it from "../../src/i18n/locales/it/translation.json";
import pt from "../../src/i18n/locales/pt/translation.json";
import "../../src/App.css";

interface ShareCardFixture {
  ready: boolean;
  /** The picture the export code path produces, as base64. */
  exportPng: () => Promise<string>;
}

declare global {
  interface Window {
    shareCard: ShareCardFixture;
  }
}

const params = new URLSearchParams(window.location.search);
const theme = params.get("theme");
if (theme === "light" || theme === "dark") {
  document.documentElement.dataset.theme = theme;
}

const TODAY = new Date(2026, 10, 7);

const isoDate = (date: Date) =>
  [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");

/** The last 20 weeks, Sunday first, like the app's heatmap. */
const heatmap = (level: (index: number) => number) => {
  const first = new Date(TODAY);
  first.setDate(first.getDate() - first.getDay() - 19 * 7);
  const days: ShareStats["heatmap"] = [];
  for (let d = new Date(first); d <= TODAY; d.setDate(d.getDate() + 1)) {
    days.push({ date: isoDate(d), level: level(days.length) });
  }
  return days;
};

const busy = (i: number) => (i % 7 === 0 || i % 11 === 3 ? 0 : (i * 7) % 5);

const DATA: Record<string, ShareStats> = {
  demo: {
    totalWords: 41_260,
    minutesSaved: 732,
    wpm: 138,
    streaks: { current: 23, longest: 58 },
    heatmap: heatmap(busy),
  },
  huge: {
    totalWords: 1_234_567_890,
    minutesSaved: 987_654,
    wpm: 1_234,
    streaks: { current: 12_345, longest: 98_765 },
    heatmap: heatmap(() => 4),
  },
  low: {
    totalWords: 12,
    minutesSaved: 0,
    wpm: null,
    streaks: { current: 1, longest: 1 },
    heatmap: heatmap((i) => (i === 139 ? 1 : 0)),
  },
};

const renderFixture = async () => {
  const lng = params.get("lng") ?? "en";
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng,
    react: { useSuspense: false },
    resources: {
      en: { translation: en },
      fr: { translation: fr },
      es: { translation: es },
      de: { translation: de },
      it: { translation: it },
      pt: { translation: pt },
    },
  });

  const stats = DATA[params.get("data") ?? "demo"];
  const rootElement = document.getElementById("root");
  if (!rootElement) throw new Error("Share card fixture is missing its root");
  ReactDOM.createRoot(rootElement).render(
    <div id="frame">
      <ShareCard stats={stats} />
    </div>,
  );
  await Promise.all([
    document.fonts.load('600 78px "Instrument Sans Variable"'),
    document.fonts.load('400 44px "Faculty Glyphic"'),
  ]);
  await document.fonts.ready;
  window.shareCard = {
    ready: true,
    exportPng: () =>
      renderShareCard(
        document.getElementById("frame")!.firstElementChild as HTMLElement,
      ),
  };
};

window.shareCard = { ready: false, exportPng: () => Promise.reject() };
void renderFixture();
