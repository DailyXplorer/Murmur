import React from "react";
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

type FixtureMode =
  | "race"
  | "listen-reject"
  | "unlisten-reject"
  | "delete-refresh"
  | "delayed-listen";

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
}

interface HistoryRequest {
  cursor: number | null;
  deferred: Deferred<PaginatedHistory>;
  snapshot: HistoryEntry[];
}

interface HistoryRaceFixture {
  addDatabaseEntry: (entry: HistoryEntry) => void;
  emit: (payload: HistoryUpdatePayload) => Promise<void>;
  emitBeforeListener: (payload: HistoryUpdatePayload) => Promise<void>;
  failNextDelete: () => void;
  historyRequestCount: () => number;
  historyRequestCursors: () => (number | null)[];
  inFlightHistoryRequests: () => number;
  listenRequestCount: () => number;
  maxInFlightHistoryRequests: () => number;
  ready: boolean;
  resolveCapturedHistoryRequest: (requestIndex: number) => void;
  resolveFirstPage: (entries: HistoryEntry[]) => void;
  resolveHistoryRequest: (
    requestIndex: number,
    entries: HistoryEntry[],
    hasMore: boolean,
  ) => void;
  resolveListener: () => void;
  settledCommitCount: () => number;
  triggerPagination: () => void;
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

const fixtureModeParam = new URLSearchParams(window.location.search).get(
  "mode",
);
const fixtureMode: FixtureMode =
  fixtureModeParam === "listen-reject" ||
  fixtureModeParam === "unlisten-reject" ||
  fixtureModeParam === "delete-refresh" ||
  fixtureModeParam === "delayed-listen"
    ? fixtureModeParam
    : "race";
const delayedListener = createDeferred<number>();
const historyRequests: HistoryRequest[] = [];
let databaseEntries: HistoryEntry[] = [
  {
    file_name: "history-404.wav",
    id: 404,
    saved: false,
    timestamp: 1_700_000_404,
    title: "History 404",
    transcription_text: "Stale listener snapshot",
  },
];
let deleteShouldFail = false;
let inFlightHistoryRequestCount = 0;
let listenerRequestCount = 0;
let maxInFlightRequestCount = 0;
let settledCommitCount = 0;
type PaginationObserverCallback = (
  entries: {
    isIntersecting: boolean;
  }[],
) => void;
let paginationObserverCallback: PaginationObserverCallback | undefined;
let unhandledRejectionCount = 0;
let unlistenAttemptCount = 0;

const addDatabaseEntry = (entry: HistoryEntry) => {
  databaseEntries = [
    entry,
    ...databaseEntries.filter((existingEntry) => existingEntry.id !== entry.id),
  ];
};

window.addEventListener("unhandledrejection", (event) => {
  unhandledRejectionCount += 1;
  event.preventDefault();
});

if (fixtureMode === "delete-refresh") {
  class ControlledIntersectionObserver {
    constructor(callback: (entries: { isIntersecting: boolean }[]) => void) {
      paginationObserverCallback = callback;
    }

    disconnect() {}

    observe() {}
  }

  Object.defineProperty(window, "IntersectionObserver", {
    value: ControlledIntersectionObserver,
  });
}

const trackHistoryRequest = (cursor: number | null) => {
  const deferred = createDeferred<PaginatedHistory>();
  const request = {
    cursor,
    deferred,
    snapshot: databaseEntries,
  };
  historyRequests.push(request);
  inFlightHistoryRequestCount += 1;
  maxInFlightRequestCount = Math.max(
    maxInFlightRequestCount,
    inFlightHistoryRequestCount,
  );
  window.historyRace.ready = true;

  return deferred.promise.finally(() => {
    inFlightHistoryRequestCount -= 1;
  });
};

mockIPC(
  (command, payload) => {
    if (command === "get_history_entries") {
      if (fixtureMode === "unlisten-reject") {
        window.historyRace.ready = true;
        return { entries: [], has_more: false };
      }

      const cursor =
        payload && "cursor" in payload && typeof payload.cursor === "number"
          ? payload.cursor
          : null;
      return trackHistoryRequest(cursor);
    }

    if (command === "delete_history_entry") {
      if (deleteShouldFail) {
        deleteShouldFail = false;
        return Promise.reject(new Error("forced delete rejection"));
      }
      return null;
    }

    if (command === "plugin:event|listen") {
      const eventName =
        payload && typeof payload === "object" && "event" in payload
          ? payload.event
          : undefined;
      if (eventName === "history-update-payload") {
        listenerRequestCount += 1;
        if (fixtureMode === "listen-reject") {
          return Promise.reject(
            new Error("forced history listener registration rejection"),
          );
        }
        if (fixtureMode === "delayed-listen") {
          return delayedListener.promise;
        }
      }
      return 1;
    }

    if (command === "plugin:event|unlisten") {
      unlistenAttemptCount += 1;
      if (fixtureMode === "unlisten-reject") {
        return Promise.reject(new Error("forced unlisten rejection"));
      }
      return null;
    }

    if (command === "plugin:event|emit") {
      return null;
    }

    throw new Error(`Unexpected Tauri command: ${command}`);
  },
  {
    shouldMockEvents:
      fixtureMode === "race" || fixtureMode === "delete-refresh",
  },
);

let root: ReactDOM.Root | undefined;

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("History race fixture is missing its root element");
}

window.historyRace = {
  addDatabaseEntry,
  emit: (payload) => events.historyUpdatePayload.emit(payload),
  emitBeforeListener: (payload) => {
    if ("entry" in payload) {
      addDatabaseEntry(payload.entry);
    }
    return events.historyUpdatePayload.emit(payload);
  },
  failNextDelete: () => {
    deleteShouldFail = true;
  },
  historyRequestCount: () => historyRequests.length,
  historyRequestCursors: () =>
    historyRequests.map((historyRequest) => historyRequest.cursor),
  inFlightHistoryRequests: () => inFlightHistoryRequestCount,
  listenRequestCount: () => listenerRequestCount,
  maxInFlightHistoryRequests: () => maxInFlightRequestCount,
  ready: false,
  resolveCapturedHistoryRequest: (requestIndex) => {
    const request = historyRequests[requestIndex];
    if (!request) {
      throw new Error(`History request ${requestIndex} was not started`);
    }
    request.deferred.resolve({ entries: request.snapshot, has_more: false });
  },
  resolveFirstPage: (entries) => {
    window.historyRace.resolveHistoryRequest(0, entries, false);
  },
  resolveHistoryRequest: (requestIndex, entries, hasMore) => {
    const request = historyRequests[requestIndex];
    if (!request) {
      throw new Error(`History request ${requestIndex} was not started`);
    }
    request.deferred.resolve({ entries, has_more: hasMore });
  },
  resolveListener: () => {
    delayedListener.resolve(1);
  },
  settledCommitCount: () => settledCommitCount,
  triggerPagination: () => {
    paginationObserverCallback?.([{ isIntersecting: true }]);
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
  root.render(
    <React.Profiler
      id="history-race"
      onRender={() => {
        if (inFlightHistoryRequestCount === 0) {
          settledCommitCount += 1;
        }
      }}
    >
      <HistorySettings />
    </React.Profiler>,
  );
};

void renderFixture();
