import React, { useEffect } from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import { events } from "../../src/bindings";
import type {
  HistoryEntry,
  HistoryUpdatePayload,
  PaginatedHistory,
} from "../../src/bindings";
import { HistorySettings } from "../../src/components/settings/history/HistorySettings";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

type FixtureMode = "race" | "unlisten-reject";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

interface HistoryRaceFixture {
  emit: (payload: HistoryUpdatePayload) => Promise<void>;
  ready: boolean;
  resolveFirstPage: (entries: HistoryEntry[]) => void;
  unhandledRejections: () => number;
  unlistenAttempts: () => number;
  unmount: () => void;
}

declare global {
  interface Window {
    historyRace: HistoryRaceFixture;
  }
}

const createDeferred = <T,>(): Deferred<T> => {
  let resolvePromise: (value: T) => void = () => undefined;
  const promise = new Promise<T>((resolve) => {
    resolvePromise = resolve;
  });

  return { promise, resolve: resolvePromise };
};

const fixtureMode: FixtureMode =
  new URLSearchParams(window.location.search).get("mode") === "unlisten-reject"
    ? "unlisten-reject"
    : "race";
const initialPage = createDeferred<PaginatedHistory>();
let unhandledRejectionCount = 0;
let unlistenAttemptCount = 0;

window.addEventListener("unhandledrejection", (event) => {
  unhandledRejectionCount += 1;
  event.preventDefault();
});

mockIPC(
  (command) => {
    if (command === "get_history_entries") {
      if (fixtureMode === "unlisten-reject") {
        return { entries: [], has_more: false };
      }
      return initialPage.promise;
    }

    if (command === "plugin:event|listen") {
      return 1;
    }

    if (command === "plugin:event|unlisten") {
      unlistenAttemptCount += 1;
      return Promise.reject(new Error("forced unlisten rejection"));
    }

    throw new Error(`Unexpected Tauri command: ${command}`);
  },
  { shouldMockEvents: fixtureMode === "race" },
);

let root: ReactDOM.Root | undefined;

const HistoryRaceHarness: React.FC = () => {
  useEffect(() => {
    void Promise.resolve().then(() => {
      window.historyRace.ready = true;
    });
  }, []);

  return <HistorySettings />;
};

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("History race fixture is missing its root element");
}

window.historyRace = {
  emit: (payload) => events.historyUpdatePayload.emit(payload),
  ready: false,
  resolveFirstPage: (entries) => {
    initialPage.resolve({ entries, has_more: false });
  },
  unhandledRejections: () => unhandledRejectionCount,
  unlistenAttempts: () => unlistenAttemptCount,
  unmount: () => root?.unmount(),
};

const renderFixture = async () => {
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: "en",
    react: { useSuspense: false },
    resources: { en: { translation: enTranslation } },
  });

  root = ReactDOM.createRoot(rootElement);
  root.render(<HistoryRaceHarness />);
};

void renderFixture();
