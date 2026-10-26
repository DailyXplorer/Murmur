import "./macos-os";
import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings, ShortcutBinding } from "../../src/bindings";
import { GlobalShortcutInput } from "../../src/components/settings/GlobalShortcutInput";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

const binding = (id: string, name: string, keys: string): ShortcutBinding => ({
  current_binding: keys,
  default_binding: keys,
  description: name,
  id,
  name,
});

const SETTINGS: AppSettings = {
  bindings: {
    transcribe: binding("transcribe", "Transcribe", "option+space"),
    cancel: binding("cancel", "Cancel", "escape"),
    paste_last: binding("paste_last", "Paste last", "ctrl+command+v"),
    meeting_toggle: binding(
      "meeting_toggle",
      "Start or stop a meeting",
      "ctrl+option+command+m",
    ),
  },
};

// A few keys of the French AZERTY layout: the key a US keyboard calls "m"
// types ",", and the key printed M is the one it calls ";".
const AZERTY_LABELS = { a: "q", q: "a", m: ",", ";": "m", v: "v" };

const bindings = SETTINGS.bindings;
if (!bindings) {
  throw new Error("Shortcut capture fixture is missing bindings");
}

declare global {
  interface Window {
    shortcutCapture: {
      calls: () => string[];
      unmount: () => void;
    };
  }
}

const calls: string[] = [];

mockIPC(
  (command, args) => {
    if (command === "get_shortcut_registration_errors") return {};
    if (command === "get_key_labels") return AZERTY_LABELS;
    // macOS records the physical key under its own name.
    if (command === "get_recorded_key_names") return {};
    if (
      command === "suspend_all_bindings" ||
      command === "resume_all_bindings"
    ) {
      calls.push(command);
      return null;
    }
    if (command === "change_binding") {
      const { id, binding: keys } = args as { id: string; binding: string };
      calls.push(`change_binding ${id} ${keys}`);
      return {
        success: true,
        binding: { ...bindings[id], current_binding: keys },
        error: null,
      };
    }
    throw new Error(`Unexpected Tauri command: ${command}`);
  },
  { shouldMockEvents: true },
);

const ShortcutCaptureFixture: React.FC = () => (
  <main className="min-h-screen bg-background px-8 py-10 text-text">
    <section className="mx-auto flex w-[720px] flex-col gap-4">
      {Object.keys(bindings).map((id) => (
        <div key={id} data-testid={`shortcut-${id}`}>
          <GlobalShortcutInput shortcutId={id} />
        </div>
      ))}
    </section>
  </main>
);

const renderFixture = async () => {
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: "en",
    react: { useSuspense: false },
    resources: { en: { translation: enTranslation } },
  });

  useSettingsStore.setState({
    defaultSettings: SETTINGS,
    isLoading: false,
    isUpdating: {},
    settings: SETTINGS,
  });

  const rootElement = document.getElementById("root");
  if (!rootElement)
    throw new Error("Shortcut capture fixture is missing its root");
  const root = ReactDOM.createRoot(rootElement);
  window.shortcutCapture = {
    calls: () => [...calls],
    unmount: () => root.unmount(),
  };
  root.render(<ShortcutCaptureFixture />);
};

void renderFixture();
