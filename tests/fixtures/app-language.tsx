import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings } from "../../src/bindings";
import "../../src/App.css";

const params = new URLSearchParams(window.location.search);
const saved = params.get("saved") ?? "";
const system = params.get("system") ?? "en-US";

const SETTINGS: AppSettings = { app_language: saved };

declare global {
  interface Window {
    /** The text React committed first, before any later language change. */
    appLanguageFirstPaint?: string;
  }
}

mockIPC((command) => {
  if (command === "get_app_settings") return SETTINGS;
  if (command === "plugin:os|locale") return system;
  return null;
});

const renderFixture = async () => {
  const [{ i18nReady }, { AppLanguageSelector }, { useSettingsStore }] =
    await Promise.all([
      import("../../src/i18n"),
      import("../../src/components/settings/AppLanguageSelector"),
      import("../../src/stores/settingsStore"),
    ]);
  // As the settings window's entry point does.
  await i18nReady;

  useSettingsStore.setState({
    defaultSettings: SETTINGS,
    isLoading: false,
    isUpdating: {},
    settings: SETTINGS,
  });

  const rootElement = document.getElementById("root");
  if (!rootElement) throw new Error("Fixture root is missing");

  ReactDOM.createRoot(rootElement).render(
    <main
      ref={(element) => {
        window.appLanguageFirstPaint ??= element?.textContent ?? undefined;
      }}
      className="min-h-screen bg-background px-8 py-10 text-text"
    >
      <AppLanguageSelector descriptionMode="inline" />
    </main>,
  );
};

void renderFixture();
