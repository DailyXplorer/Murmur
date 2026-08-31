import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { mockIPC } from "@tauri-apps/api/mocks";
import AccessibilityPermissions from "../../src/components/AccessibilityPermissions";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

const unhandledRejections: string[] = [];
let permissionChecks = 0;

window.addEventListener("unhandledrejection", (event) => {
  event.preventDefault();
  unhandledRejections.push(String(event.reason));
});

mockIPC((command) => {
  if (command === "plugin:macos-permissions|check_accessibility_permission") {
    permissionChecks += 1;
    return Promise.reject(new Error("Permission bridge unavailable"));
  }

  throw new Error(`Unexpected Tauri command: ${command}`);
});

declare global {
  interface Window {
    accessibilityFailureFixture: {
      permissionChecks: () => number;
      unhandledRejections: () => string[];
    };
  }
}

window.accessibilityFailureFixture = {
  permissionChecks: () => permissionChecks,
  unhandledRejections: () => [...unhandledRejections],
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
    <main data-testid="accessibility-fixture">
      <AccessibilityPermissions />
    </main>,
  );
};

void renderFixture();
