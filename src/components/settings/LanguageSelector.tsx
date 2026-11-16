import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { ResetButton } from "../ui/ResetButton";
import { useSettings } from "../../hooks/useSettings";
import { useLanguageName } from "../../hooks/useLanguageName";
import {
  recognitionLanguage,
  SELECTABLE_LANGUAGES,
  supportsLanguageCode,
} from "../../lib/constants/languages";
import { LanguageMenu } from "./LanguageMenu";

interface LanguageSelectorProps {
  grouped?: boolean;
  supportedLanguages?: string[];
  // Whether the transcription service can auto-detect language.
  supportsLanguageDetection?: boolean;
}

// Resolve the canonical base code shown by the picker when the service exposes
// a restricted language list.
const effectiveLanguage = (
  intent: string,
  supported: string[],
  supportsDetection: boolean,
): string => {
  if (supported.length === 0) return intent;
  if (intent !== "auto" && supportsLanguageCode(supported, intent))
    return intent;
  if (supportsDetection) return "auto";
  if (supportsLanguageCode(supported, "en")) return "en";
  return recognitionLanguage(supported[0]);
};

/** Settings control for choosing a recognition language from a portaled list. */
export const LanguageSelector: React.FC<LanguageSelectorProps> = ({
  grouped = false,
  supportedLanguages,
  supportsLanguageDetection = true,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, resetSetting, isUpdating } = useSettings();
  const isLanguageUpdating = isUpdating("selected_language");

  // The persisted *intent* (auto | code). What's actually used/shown is the
  // effective value resolved against the transcription service capabilities.
  const intent = getSetting("selected_language") || "auto";
  const selectedLanguage = effectiveLanguage(
    intent,
    supportedLanguages ?? [],
    supportsLanguageDetection,
  );

  const availableLanguages = useMemo(() => {
    if (!supportedLanguages || supportedLanguages.length === 0)
      return SELECTABLE_LANGUAGES;
    return SELECTABLE_LANGUAGES.filter((lang) =>
      lang.value === "auto"
        ? supportsLanguageDetection
        : supportsLanguageCode(supportedLanguages, lang.value),
    );
  }, [supportedLanguages, supportsLanguageDetection]);

  const languageName = useLanguageName();
  const options = useMemo(
    () =>
      availableLanguages.map((language) => ({
        ...language,
        name: languageName(language.value),
      })),
    [availableLanguages, languageName],
  );

  const handleReset = async () => {
    await resetSetting("selected_language");
  };

  return (
    <SettingContainer
      title={t("settings.general.language.title")}
      description={t("settings.general.language.description")}
      grouped={grouped}
    >
      <div className="flex min-w-0 items-center space-x-1">
        <LanguageMenu
          options={options}
          selected={selectedLanguage}
          selectedName={languageName(selectedLanguage)}
          onSelect={(code) => updateSetting("selected_language", code)}
          disabled={isLanguageUpdating}
        />
        <ResetButton
          className="shrink-0"
          onClick={handleReset}
          disabled={isLanguageUpdating}
          ariaLabel={`${t("common.reset")}: ${t("settings.general.language.title")}`}
        />
      </div>
      {isLanguageUpdating && (
        <div className="absolute inset-0 bg-mid-gray/10 rounded flex items-center justify-center">
          <div className="w-4 h-4 border-2 border-logo-primary border-t-transparent rounded-full animate-spin"></div>
        </div>
      )}
    </SettingContainer>
  );
};
