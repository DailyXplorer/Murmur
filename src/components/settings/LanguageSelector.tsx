import React, {
  useState,
  useRef,
  useEffect,
  useMemo,
  useCallback,
  useId,
} from "react";
import { CaretDownIcon } from "@phosphor-icons/react/dist/csr/CaretDown";
import { useTranslation } from "react-i18next";
import { SettingContainer } from "../ui/SettingContainer";
import { ResetButton } from "../ui/ResetButton";
import { FloatingPanel } from "../ui/FloatingPanel";
import { Input } from "../ui/Input";
import {
  FIELD_SIZE_CLASSES,
  FIELD_SURFACE_CLASSES,
} from "../ui/controlClasses";
import { useSettings } from "../../hooks/useSettings";
import {
  getLanguageLabel,
  recognitionLanguage,
  SELECTABLE_LANGUAGES,
  supportsLanguageCode,
} from "../../lib/constants/languages";

interface LanguageSelectorProps {
  descriptionMode?: "inline" | "tooltip";
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
  descriptionMode = "tooltip",
  grouped = false,
  supportedLanguages,
  supportsLanguageDetection = true,
}) => {
  const { t, i18n } = useTranslation();
  const { getSetting, updateSetting, resetSetting, isUpdating } = useSettings();
  const [isOpen, setIsOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const dropdownRef = useRef<HTMLButtonElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const languageListRef = useRef<HTMLDivElement>(null);
  const triggerId = useId();
  const menuId = useId();
  const isLanguageUpdating = isUpdating("selected_language");
  const isLanguageMenuOpen = isOpen && !isLanguageUpdating;

  // The persisted *intent* (auto | code). What's actually used/shown is the
  // effective value resolved against the transcription service capabilities.
  const intent = getSetting("selected_language") || "auto";
  const selectedLanguage = effectiveLanguage(
    intent,
    supportedLanguages ?? [],
    supportsLanguageDetection,
  );

  useEffect(() => {
    if (isLanguageMenuOpen && searchInputRef.current) {
      searchInputRef.current.focus();
    }
  }, [isLanguageMenuOpen]);

  const availableLanguages = useMemo(() => {
    if (!supportedLanguages || supportedLanguages.length === 0)
      return SELECTABLE_LANGUAGES;
    return SELECTABLE_LANGUAGES.filter((lang) =>
      lang.value === "auto"
        ? supportsLanguageDetection
        : supportsLanguageCode(supportedLanguages, lang.value),
    );
  }, [supportedLanguages, supportsLanguageDetection]);

  // Each language in the app's language, with the English name as the
  // fallback for a code the browser cannot name ("jw").
  const displayNames = useMemo(
    () => new Intl.DisplayNames([i18n.language], { type: "language" }),
    [i18n.language],
  );
  const languageName = useCallback(
    (code: string) => {
      const englishName = getLanguageLabel(code);
      if (code === "auto" || !englishName) {
        return t("settings.general.language.auto");
      }
      const name = displayNames.of(code);
      if (!name || name === code) return englishName;
      return name.charAt(0).toLocaleUpperCase(i18n.language) + name.slice(1);
    },
    [displayNames, i18n.language, t],
  );

  const filteredLanguages = useMemo(() => {
    const query = searchQuery.toLowerCase();
    return availableLanguages
      .map((language) => ({ ...language, name: languageName(language.value) }))
      .filter(
        (language) =>
          language.name.toLowerCase().includes(query) ||
          language.label.toLowerCase().includes(query),
      );
  }, [searchQuery, availableLanguages, languageName]);
  const initialTabStopValue =
    filteredLanguages.find((language) => language.value === selectedLanguage)
      ?.value ?? filteredLanguages[0]?.value;

  const selectedLanguageName = languageName(selectedLanguage);

  const handleLanguageSelect = async (languageCode: string) => {
    await updateSetting("selected_language", languageCode);
    setIsOpen(false);
    setSearchQuery("");
    requestAnimationFrame(() => dropdownRef.current?.focus());
  };

  const handleReset = async () => {
    await resetSetting("selected_language");
  };

  const handleToggle = () => {
    if (isLanguageUpdating) return;
    setIsOpen(!isOpen);
  };

  const handleDismiss = useCallback(() => {
    setIsOpen(false);
    setSearchQuery("");
  }, []);

  const handleSearchChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    setSearchQuery(event.target.value);
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" && filteredLanguages.length > 0) {
      handleLanguageSelect(filteredLanguages[0].value);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      languageListRef.current
        ?.querySelector<HTMLElement>('[role="option"]:not([disabled])')
        ?.focus();
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      handleDismiss();
      dropdownRef.current?.focus();
    }
  };

  return (
    <SettingContainer
      title={t("settings.general.language.title")}
      description={t("settings.general.language.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <div className="flex min-w-0 items-center space-x-1">
        <div className="relative min-w-0 flex-1">
          <button
            ref={dropdownRef}
            id={triggerId}
            type="button"
            data-control="field"
            className={`${FIELD_SIZE_CLASSES} ${FIELD_SURFACE_CLASSES} flex w-full min-w-[200px] items-center justify-between transition-[background-color,border-color] duration-150 ${
              isLanguageUpdating
                ? "opacity-50 cursor-not-allowed"
                : "hover:bg-logo-primary/10 cursor-pointer hover:border-logo-primary"
            }`}
            onClick={handleToggle}
            disabled={isLanguageUpdating}
            aria-haspopup="listbox"
            aria-expanded={isLanguageMenuOpen}
            aria-controls={isLanguageMenuOpen ? menuId : undefined}
          >
            <span className="truncate">{selectedLanguageName}</span>
            <CaretDownIcon
              size={14}
              className={`ms-2 transition-transform duration-200 ${
                isLanguageMenuOpen ? "transform rotate-180" : ""
              }`}
            />
          </button>

          <FloatingPanel
            open={isLanguageMenuOpen}
            anchorRef={dropdownRef}
            onDismiss={handleDismiss}
            className="flex flex-col overflow-hidden rounded-md border border-mid-gray/80 bg-background shadow-lg"
          >
            <div className="p-2 border-b border-mid-gray/80">
              <Input
                ref={searchInputRef}
                type="text"
                value={searchQuery}
                onChange={handleSearchChange}
                onKeyDown={handleKeyDown}
                placeholder={t("settings.general.language.searchPlaceholder")}
                className="w-full"
              />
            </div>

            <div
              ref={languageListRef}
              id={menuId}
              role="listbox"
              aria-labelledby={triggerId}
              className="min-h-0 flex-1 overflow-y-auto"
            >
              {filteredLanguages.length === 0 ? (
                <div className="px-2 py-2 text-sm text-mid-gray text-center">
                  {t("settings.general.language.noResults")}
                </div>
              ) : (
                filteredLanguages.map((language) => (
                  <button
                    key={language.value}
                    type="button"
                    role="option"
                    aria-selected={selectedLanguage === language.value}
                    tabIndex={language.value === initialTabStopValue ? 0 : -1}
                    className={`min-h-control w-full px-3 py-1 text-sm font-normal text-start hover:bg-logo-primary/10 transition-colors duration-150 ${
                      selectedLanguage === language.value
                        ? "bg-logo-primary/20 text-accent-text"
                        : ""
                    }`}
                    onClick={() => handleLanguageSelect(language.value)}
                  >
                    <div className="flex items-center justify-between">
                      <span className="truncate">{language.name}</span>
                    </div>
                  </button>
                ))
              )}
            </div>
          </FloatingPanel>
        </div>
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
