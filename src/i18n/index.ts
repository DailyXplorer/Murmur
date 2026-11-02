import i18n, { type BackendModule } from "i18next";
import { initReactI18next } from "react-i18next";
import { locale } from "@tauri-apps/plugin-os";
import { LANGUAGE_METADATA } from "./languages";
import { commands } from "@/bindings";

// Each language is its own chunk: a window loads English, the fallback,
// and the one it shows.
const localeLoaders = import.meta.glob<{ default: Record<string, unknown> }>(
  "./locales/*/translation.json",
);

const localeLoader = (code: string) =>
  localeLoaders[`./locales/${code}/translation.json`];

/** Every `changeLanguage` waits for its bundle through this backend. */
const localeBackend: BackendModule = {
  type: "backend",
  init: () => undefined,
  read: (language, _namespace, callback) => {
    const load = localeLoader(language);
    if (!load) {
      callback(new Error(`No translations for "${language}"`), false);
      return;
    }
    load().then(
      (module) => callback(null, module.default),
      (error: unknown) => callback(error as Error, false),
    );
  },
};

export const SUPPORTED_LANGUAGES = Object.keys(localeLoaders)
  .flatMap(
    (path) => path.match(/\.\/locales\/(.+)\/translation\.json/)?.[1] ?? [],
  )
  .map((code) => {
    const meta = LANGUAGE_METADATA[code];
    if (!meta) {
      console.warn(`Missing metadata for locale "${code}" in languages.ts`);
      return { code, name: code, nativeName: code, priority: undefined };
    }
    return {
      code,
      name: meta.name,
      nativeName: meta.nativeName,
      priority: meta.priority,
    };
  })
  .sort((a, b) => {
    if (a.priority !== undefined && b.priority !== undefined) {
      return a.priority - b.priority;
    }
    if (a.priority !== undefined) return -1;
    if (b.priority !== undefined) return 1;
    return a.name.localeCompare(b.name);
  });

export type SupportedLanguageCode = string;

export const getSupportedLanguage = (
  langCode: string | null | undefined,
): SupportedLanguageCode | null => {
  if (!langCode) return null;

  const normalized = langCode.toLowerCase().replace(/_/g, "-");
  const language = normalized.split("-")[0];
  const find = (code: string) =>
    SUPPORTED_LANGUAGES.find((lang) => lang.code.toLowerCase() === code);

  return (find(normalized) ?? find(language))?.code ?? null;
};

i18n.on("languageChanged", (lng) => {
  document.documentElement.lang = lng;
});

const initialized = i18n
  .use(localeBackend)
  .use(initReactI18next)
  .init({
    lng: "en",
    fallbackLng: "en",
    interpolation: {
      escapeValue: false, // React already escapes values
    },
    react: {
      useSuspense: false,
    },
  });

export const syncLanguageFromSettings = async () => {
  try {
    const result = await commands.getAppSettings();
    if (result.status === "ok" && result.data.app_language) {
      const supported = getSupportedLanguage(result.data.app_language);
      if (supported && supported !== i18n.language) {
        await i18n.changeLanguage(supported);
      }
    } else {
      const systemLocale = await locale();
      const supported = getSupportedLanguage(systemLocale);
      if (supported && supported !== i18n.language) {
        await i18n.changeLanguage(supported);
      }
    }
  } catch (e) {
    console.warn("Failed to sync language from settings:", e);
  }
};

/** Resolves once the window's language is loaded and active, so a first
 * render never shows English to a user who chose another language. */
export const i18nReady = initialized.then(syncLanguageFromSettings);

export default i18n;
