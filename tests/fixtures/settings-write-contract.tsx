import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings, Theme } from "../../src/bindings";
import { AccentColorSelector } from "../../src/components/settings/AccentColorSelector";
import { AppLanguageSelector } from "../../src/components/settings/AppLanguageSelector";
import { AutoSubmit } from "../../src/components/settings/AutoSubmit";
import { ThemeSelector } from "../../src/components/settings/ThemeSelector";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

const INITIAL_SETTINGS: AppSettings = {
  app_language: "en",
  accent_color: "pink",
  auto_submit: false,
  auto_submit_key: "enter",
  history_limit: 50,
  theme: "system",
};

const calls: string[] = [];
const failedCommands = new Set<string>();
const observedAccentColors: string[] = [];
let deferredCommand: string | null = null;
let rejectDeferredCommand: ((reason?: unknown) => void) | null = null;

const accentObserver = new MutationObserver(() => {
  const accentColor = document.documentElement.dataset.accentColor;
  if (accentColor) observedAccentColors.push(accentColor);
});

mockIPC((command) => {
  calls.push(command);

  if (command === deferredCommand) {
    return new Promise((_resolve, reject) => {
      rejectDeferredCommand = reject;
    });
  }

  if (failedCommands.has(command)) {
    return Promise.reject(`Rejected ${command}`);
  }

  return null;
});

const i18nReady = i18n.use(initReactI18next).init({
  fallbackLng: "en",
  interpolation: { escapeValue: false },
  lng: "en",
  react: { useSuspense: false },
  resources: { en: { translation: enTranslation } },
});

const reset = async () => {
  await i18nReady;
  await i18n.changeLanguage("en");
  calls.length = 0;
  failedCommands.clear();
  observedAccentColors.length = 0;
  deferredCommand = null;
  rejectDeferredCommand = null;
  delete document.documentElement.dataset.theme;
  document.documentElement.dataset.accentColor = "pink";
  localStorage.removeItem("murmur.theme");
  localStorage.setItem("murmur.accent-color", "pink");
  useSettingsStore.setState({
    isLoading: false,
    isUpdating: {},
    settings: { ...INITIAL_SETTINGS },
  });
};

const waitForDeferredCommand = async () => {
  while (!rejectDeferredCommand) {
    await Promise.resolve();
  }
};

declare global {
  interface Window {
    settingsWriteContract: {
      calls: () => string[];
      currentAppearance: () => {
        accentColor: string | undefined;
        appLanguage: string | undefined;
        observedAccentColors: string[];
        renderedLanguage: string;
        theme: Theme | undefined;
      };
      failCommand: (command: string) => void;
      reset: () => Promise<void>;
      runRollbackProbe: () => Promise<{
        historyLimit: number | undefined;
        result: boolean;
        theme: Theme | undefined;
      }>;
      runSameKeyOrderingProbe: () => Promise<{
        callsBeforeRelease: string[];
        finalTheme: Theme | undefined;
        firstResult: boolean;
        isUpdating: boolean;
        secondResult: boolean;
      }>;
      updateTheme: (theme: Theme) => Promise<boolean>;
    };
  }
}

window.settingsWriteContract = {
  calls: () => [...calls],
  currentAppearance: () => ({
    accentColor: useSettingsStore.getState().settings?.accent_color,
    appLanguage: useSettingsStore.getState().settings?.app_language,
    observedAccentColors: [...observedAccentColors],
    renderedLanguage: i18n.language,
    theme: useSettingsStore.getState().settings?.theme,
  }),
  failCommand: (command) => failedCommands.add(command),
  reset,
  runRollbackProbe: async () => {
    deferredCommand = "change_theme_setting";
    const update = useSettingsStore.getState().updateSetting("theme", "dark");
    await waitForDeferredCommand();

    useSettingsStore.setState((state) => ({
      settings: state.settings
        ? { ...state.settings, history_limit: 200 }
        : null,
    }));

    rejectDeferredCommand?.("Theme persistence failed");
    const result = await update;
    const settings = useSettingsStore.getState().settings;

    return {
      historyLimit: settings?.history_limit,
      result,
      theme: settings?.theme,
    };
  },
  runSameKeyOrderingProbe: async () => {
    deferredCommand = "change_theme_setting";
    const firstUpdate = useSettingsStore
      .getState()
      .updateSetting("theme", "dark");
    await waitForDeferredCommand();

    const secondUpdate = useSettingsStore
      .getState()
      .updateSetting("theme", "light");
    await Promise.resolve();
    const callsBeforeRelease = [...calls];

    deferredCommand = null;
    rejectDeferredCommand?.("First theme persistence failed");
    const [firstResult, secondResult] = await Promise.all([
      firstUpdate,
      secondUpdate,
    ]);

    return {
      callsBeforeRelease,
      finalTheme: useSettingsStore.getState().settings?.theme,
      firstResult,
      isUpdating: useSettingsStore.getState().isUpdatingKey("theme"),
      secondResult,
    };
  },
  updateTheme: (theme) =>
    useSettingsStore.getState().updateSetting("theme", theme),
};

accentObserver.observe(document.documentElement, {
  attributeFilter: ["data-accent-color"],
  attributes: true,
});

const renderFixture = async () => {
  await reset();
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <main className="flex flex-col gap-4 p-8">
      <section data-testid="auto-submit">
        <AutoSubmit />
      </section>
      <section data-testid="theme">
        <ThemeSelector />
      </section>
      <section data-testid="app-language">
        <AppLanguageSelector />
      </section>
      <section data-testid="accent-color">
        <AccentColorSelector />
      </section>
    </main>,
  );
};

void renderFixture();
