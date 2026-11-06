import "./macos-os";
import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import { events } from "../../src/bindings";
import type {
  AppSettings,
  HeatmapDay,
  InsightsSummary,
} from "../../src/bindings";
import { Sidebar } from "../../src/components/Sidebar";
import { InsightsSettings } from "../../src/components/settings/insights/InsightsSettings";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

interface InsightsFixture {
  ready: boolean;
  requests: () => number;
  publish: (summary: InsightsSummary) => Promise<void>;
}

declare global {
  interface Window {
    insights: InsightsFixture;
  }
}

const params = new URLSearchParams(window.location.search);
const theme = params.get("theme");
if (theme === "light" || theme === "dark") {
  document.documentElement.dataset.theme = theme;
}

const TODAY = new Date(2026, 9, 15);

const isoDate = (date: Date) =>
  [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");

const heatmap = (countFor: (index: number) => number): HeatmapDay[] => {
  const first = new Date(TODAY);
  first.setDate(first.getDate() - first.getDay() - 19 * 7);
  const days: HeatmapDay[] = [];
  for (let d = new Date(first); d <= TODAY; d.setDate(d.getDate() + 1)) {
    days.push({ date: isoDate(d), count: countFor(days.length), level: 0 });
  }
  const max = Math.max(...days.map((day) => day.count));
  return days.map((day) => ({
    ...day,
    level: day.count === 0 ? 0 : Math.min(4, Math.ceil((day.count * 4) / max)),
  }));
};

const DEMO_SUMMARY: InsightsSummary = {
  total_dictations: 1_284,
  total_words: 48_210,
  words_per_minute: 142,
  minutes_saved: 865,
  categories: [
    { category: "ai_prompts", count: 96 },
    { category: "coding", count: 54 },
    { category: "work_messages", count: 31 },
    { category: "emails", count: 12 },
    { category: "other", count: 7 },
  ],
  categorized_dictations: 200,
  apps_used: 14,
  streaks: { current: 12, longest: 31 },
  heatmap: heatmap((i) =>
    i % 7 === 0 || i % 11 === 3 ? 0 : ((i * 7) % 13) + 1,
  ),
};

const EMPTY_SUMMARY: InsightsSummary = {
  total_dictations: 0,
  total_words: 0,
  words_per_minute: null,
  minutes_saved: 0,
  categories: [],
  categorized_dictations: 0,
  apps_used: 0,
  streaks: { current: 0, longest: 0 },
  heatmap: heatmap(() => 0),
};

let summary = params.get("data") === "empty" ? EMPTY_SUMMARY : DEMO_SUMMARY;
let requestCount = 0;

mockIPC(
  (command) => {
    if (command === "get_insights") {
      requestCount += 1;
      window.insights.ready = true;
      return summary;
    }
    throw new Error(`Unexpected Tauri command: ${command}`);
  },
  { shouldMockEvents: true },
);

window.insights = {
  ready: false,
  requests: () => requestCount,
  publish: (next) => {
    summary = next;
    return events.insightsUpdated.emit(null);
  },
};

const InsightsFixture: React.FC = () => (
  <div className="flex h-screen select-none bg-background text-text">
    <Sidebar activeSection="insights" onSectionChange={() => undefined} />
    <div className="flex-1 overflow-y-auto">
      <div className="flex flex-col items-center gap-4 p-4">
        <InsightsSettings />
      </div>
    </div>
  </div>
);

const renderFixture = async () => {
  useSettingsStore.setState({
    isLoading: false,
    isUpdating: {},
    settings: { debug_mode: false } as AppSettings,
  });
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: "en",
    react: { useSuspense: false },
    resources: { en: { translation: enTranslation } },
  });

  const rootElement = document.getElementById("root");
  if (!rootElement) throw new Error("Insights fixture is missing its root");
  ReactDOM.createRoot(rootElement).render(<InsightsFixture />);
};

void renderFixture();
