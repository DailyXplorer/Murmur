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
  const { t } = useTranslation();
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

  const filteredLanguages = useMemo(
    () =>
      availableLanguages.filter((language) =>
        language.label.toLowerCase().includes(searchQuery.toLowerCase()),
      ),
    [searchQuery, availableLanguages],
  );
  const initialTabStopValue =
    filteredLanguages.find((language) => language.value === selectedLanguage)
      ?.value ?? filteredLanguages[0]?.value;

  const selectedLanguageName =
    getLanguageLabel(selectedLanguage) || t("settings.general.language.auto");

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
      // Select first filtered language on Enter
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
      <div className="flex items-center space-x-1">
        <div className="relative">
          <button
            ref={dropdownRef}
            id={triggerId}
            type="button"
            className={`px-2 py-1 text-sm font-normal bg-mid-gray/10 border border-mid-gray/80 rounded min-w-[200px] text-start flex items-center justify-between transition-[background-color,border-color] duration-150 ${
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
            className="flex flex-col overflow-hidden rounded border border-mid-gray/80 bg-background shadow-lg"
          >
            {/* Search input */}
            <div className="p-2 border-b border-mid-gray/80">
              <input
                ref={searchInputRef}
                type="text"
                value={searchQuery}
                onChange={handleSearchChange}
                onKeyDown={handleKeyDown}
                placeholder={t("settings.general.language.searchPlaceholder")}
                className="w-full px-2 py-1 text-sm bg-mid-gray/10 border border-mid-gray/40 rounded focus:outline-none focus:ring-1 focus:ring-logo-primary focus:border-logo-primary"
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
                    className={`w-full px-2 py-1 text-sm font-normal text-start hover:bg-logo-primary/10 transition-colors duration-150 ${
                      selectedLanguage === language.value
                        ? "bg-logo-primary/20 text-logo-primary"
                        : ""
                    }`}
                    onClick={() => handleLanguageSelect(language.value)}
                  >
                    <div className="flex items-center justify-between">
                      <span className="truncate">{language.label}</span>
                    </div>
                  </button>
                ))
              )}
            </div>
          </FloatingPanel>
        </div>
        <ResetButton onClick={handleReset} disabled={isLanguageUpdating} />
      </div>
      {isLanguageUpdating && (
        <div className="absolute inset-0 bg-mid-gray/10 rounded flex items-center justify-center">
          <div className="w-4 h-4 border-2 border-logo-primary border-t-transparent rounded-full animate-spin"></div>
        </div>
      )}
    </SettingContainer>
  );
};
