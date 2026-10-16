import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import AccessibilityOnboarding from "../../src/components/onboarding/AccessibilityOnboarding";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

const openedUrls: string[] = [];
let microphoneGranted = false;

mockIPC((command, args) => {
  switch (command) {
    case "plugin:macos-permissions|check_accessibility_permission":
      return true;
    case "plugin:macos-permissions|check_microphone_permission":
      return microphoneGranted;
    case "plugin:macos-permissions|request_microphone_permission":
    case "initialize_enigo":
    case "initialize_shortcuts":
      return null;
    case "get_available_microphones":
    case "get_available_output_devices":
      return [];
    case "plugin:opener|open_url":
      openedUrls.push(String((args as { url: string }).url));
      return null;
    default:
      throw new Error(`Unexpected Tauri command: ${command}`);
  }
});

declare global {
  interface Window {
    microphoneDeniedFixture: {
      grantMicrophone: () => void;
      openedUrls: () => string[];
    };
  }
}

window.microphoneDeniedFixture = {
  grantMicrophone: () => {
    microphoneGranted = true;
  },
  openedUrls: () => [...openedUrls],
};

const renderFixture = async () => {
  await i18n.use(initReactI18next).init({
    fallbackLng: "en",
    interpolation: { escapeValue: false },
    lng: "en",
    react: { useSuspense: false },
    resources: { en: { translation: enTranslation } },
  });

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <main data-testid="microphone-denied-fixture">
      <AccessibilityOnboarding onComplete={() => {}} />
    </main>,
  );
};

void renderFixture();
