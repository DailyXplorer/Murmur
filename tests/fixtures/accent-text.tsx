import React from "react";
import ReactDOM from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import type { AppSettings } from "../../src/bindings";
import { LanguageSelector } from "../../src/components/settings/LanguageSelector";
import { Button } from "../../src/components/ui/Button";
import { IconButton } from "../../src/components/ui/IconButton";
import { MarkdownContent } from "../../src/components/whats-new/MarkdownContent";
import { speakerColor } from "../../src/lib/meetings/speakerColors";
import { useSettingsStore } from "../../src/stores/settingsStore";
import enTranslation from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

const TEST_SETTINGS = {
  selected_language: "auto",
} as AppSettings;

/** Every accent-coloured text and focus ring that sits on the page. */
const AccentTextFixture: React.FC = () => (
  <main className="flex min-h-screen flex-col gap-6 bg-background p-8 text-text">
    <Button>Focus me</Button>
    <IconButton label="Pinned" active>
      P
    </IconButton>
    <div data-testid="markdown">
      <MarkdownContent markdown="Read the [release notes](https://example.com)." />
    </div>
    <ol data-testid="speakers" className="text-sm font-medium">
      {Array.from({ length: 8 }, (_, index) => (
        <li key={index} style={{ color: speakerColor(index) }}>
          Speaker {index}
        </li>
      ))}
    </ol>
    <LanguageSelector />
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
    defaultSettings: TEST_SETTINGS,
    isLoading: false,
    isUpdating: {},
    settings: TEST_SETTINGS,
  });

  const rootElement = document.getElementById("root");
  if (!rootElement) throw new Error("Fixture root is missing");

  ReactDOM.createRoot(rootElement).render(<AccentTextFixture />);
};

void renderFixture();
