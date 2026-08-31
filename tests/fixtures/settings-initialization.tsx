import React from "react";
import ReactDOM from "react-dom/client";
import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings } from "../../src/bindings";
import { useSettings } from "../../src/hooks/useSettings";
import {
  createSettingsChangedListenerLifecycle,
  useSettingsStore,
} from "../../src/stores/settingsStore";

declare global {
  interface Window {
    settingsInitialization: {
      customSounds: number;
      defaultSettings: number;
      listeners: number;
      settings: number;
      unhandledRejections: number;
      emitSettingsChanged: () => Promise<void>;
      runSettingsEventDuringWrite: () => Promise<{
        listeners: number;
        settingsReadsAfterRelease: number;
        settingsReadsBeforeRelease: number;
        theme: string | undefined;
        updateResult: boolean;
      }>;
      runHmrLifecycleRace: () => Promise<{
        activeListeners: number;
        aEvents: number;
        cEvents: number;
        listenerRegistrations: number;
        maximumActiveListeners: number;
      }>;
    };
  }
}

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
  theme: "system",
  transcription_provider: "gemini",
};

const respondAfterDelay = <Value,>(value: Value): Promise<Value> =>
  new Promise((resolve) => {
    window.setTimeout(() => resolve(value), 25);
  });

type SettingsChangedEvent = { payload: { setting?: string } };
type SettingsChangedHandler = (event: SettingsChangedEvent) => void;

const createDeferred = <Value,>() => {
  let resolvePromise: ((value: Value | PromiseLike<Value>) => void) | undefined;
  const promise = new Promise<Value>((resolve) => {
    resolvePromise = resolve;
  });

  return {
    promise,
    resolve: (value: Value) => {
      if (resolvePromise === undefined) {
        throw new Error("Deferred promise was not initialized");
      }
      resolvePromise(value);
    },
  };
};

const flushMicrotasks = async () => {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
};

let backendSettings: AppSettings = { ...TEST_SETTINGS };
let deferThemeWrite = false;
let themeWriteCalls = 0;
const themeWrite = createDeferred<null>();

const waitFor = async (predicate: () => boolean) => {
  while (!predicate()) {
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
  }
};

const SettingsConsumer: React.FC = () => {
  const { isLoading } = useSettings();

  return <output>{isLoading ? "loading" : "ready"}</output>;
};

const configuredListenerFailures = Number(
  new URLSearchParams(window.location.search).get("listener-failures") ?? 0,
);

window.settingsInitialization = {
  customSounds: 0,
  defaultSettings: 0,
  listeners: 0,
  settings: 0,
  unhandledRejections: 0,
  emitSettingsChanged: () => emit("settings-changed", { setting: "theme" }),
  runSettingsEventDuringWrite: async () => {
    deferThemeWrite = true;
    const update = useSettingsStore.getState().updateSetting("theme", "dark");
    await waitFor(() => themeWriteCalls === 1);

    await emit("settings-changed", { setting: "theme" });
    await flushMicrotasks();
    const settingsReadsBeforeRelease = window.settingsInitialization.settings;

    backendSettings = { ...backendSettings, theme: "dark" };
    themeWrite.resolve(null);
    const updateResult = await update;
    await waitFor(
      () =>
        window.settingsInitialization.settings === 2 &&
        useSettingsStore.getState().settings?.theme === "dark",
    );

    return {
      listeners: window.settingsInitialization.listeners,
      settingsReadsAfterRelease: window.settingsInitialization.settings,
      settingsReadsBeforeRelease,
      theme: useSettingsStore.getState().settings?.theme,
      updateResult,
    };
  },
  runHmrLifecycleRace: async () => {
    const activeListeners = new Set<SettingsChangedHandler>();
    let maximumActiveListeners = 0;
    const addActiveListener = (handler: SettingsChangedHandler) => {
      activeListeners.add(handler);
      maximumActiveListeners = Math.max(
        maximumActiveListeners,
        activeListeners.size,
      );
    };
    let aEvents = 0;
    let cEvents = 0;
    let aHandler: SettingsChangedHandler | undefined;
    let listenerRegistrations = 0;
    const aListenerRegistration = createDeferred<() => Promise<void>>();
    const aUnlistenFinished = createDeferred<void>();

    const generationA = createSettingsChangedListenerLifecycle({
      listen: (handler) => {
        aHandler = handler;
        listenerRegistrations += 1;
        return aListenerRegistration.promise;
      },
      onSettingsChanged: () => {
        aEvents += 1;
      },
    });
    const generationAInitialization = generationA.initialize();
    await flushMicrotasks();

    const generationACleanup = generationA.dispose();
    const generationB = createSettingsChangedListenerLifecycle({
      previousCleanup: generationACleanup,
      listen: async () => {
        throw new Error("Disposed generation B must not register a listener");
      },
      onSettingsChanged: () => undefined,
    });
    const generationBCleanup = generationB.dispose();
    const generationC = createSettingsChangedListenerLifecycle({
      previousCleanup: generationBCleanup,
      listen: async (handler) => {
        listenerRegistrations += 1;
        addActiveListener(handler);
        return async () => {
          activeListeners.delete(handler);
        };
      },
      onSettingsChanged: () => {
        cEvents += 1;
      },
    });
    const generationCInitialization = generationC.initialize();

    await flushMicrotasks();

    if (aHandler === undefined) {
      throw new Error("Generation A did not begin listener registration");
    }

    addActiveListener(aHandler);
    aListenerRegistration.resolve(async () => {
      await aUnlistenFinished.promise;
      activeListeners.delete(aHandler);
    });
    await flushMicrotasks();

    aUnlistenFinished.resolve(undefined);
    await Promise.all([
      generationAInitialization,
      generationACleanup,
      generationBCleanup,
      generationCInitialization,
    ]);

    for (const handler of activeListeners) {
      handler({ payload: { setting: "theme" } });
    }

    return {
      activeListeners: activeListeners.size,
      aEvents,
      cEvents,
      listenerRegistrations,
      maximumActiveListeners,
    };
  },
};

window.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  window.settingsInitialization.unhandledRejections += 1;
});

mockIPC(
  (command) => {
    switch (command) {
      case "get_app_settings":
        window.settingsInitialization.settings += 1;
        return respondAfterDelay({ ...backendSettings });
      case "get_default_settings":
        window.settingsInitialization.defaultSettings += 1;
        return respondAfterDelay(TEST_SETTINGS);
      case "check_custom_sounds":
        window.settingsInitialization.customSounds += 1;
        return respondAfterDelay({ start: false, stop: false });
      case "change_theme_setting":
        themeWriteCalls += 1;
        return deferThemeWrite ? themeWrite.promise : null;
      default:
        throw new Error(`Unexpected Tauri command: ${command}`);
    }
  },
  { shouldMockEvents: true },
);

type TauriInvoke = (
  command: string,
  args?: unknown,
  options?: unknown,
) => Promise<unknown>;

const tauriInternals = window.__TAURI_INTERNALS__ as unknown as {
  invoke: TauriInvoke;
};
const invoke = tauriInternals.invoke;
let remainingListenerFailures =
  Number.isInteger(configuredListenerFailures) && configuredListenerFailures > 0
    ? configuredListenerFailures
    : 0;

tauriInternals.invoke = async (command, args, options) => {
  if (command === "plugin:event|listen") {
    window.settingsInitialization.listeners += 1;

    if (remainingListenerFailures > 0) {
      remainingListenerFailures -= 1;
      throw new Error("Expected listener registration failure");
    }
  }

  return invoke(command, args, options);
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <SettingsConsumer />
    <SettingsConsumer />
    <SettingsConsumer />
  </React.StrictMode>,
);
