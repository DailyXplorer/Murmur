import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { getLanguageLabel } from "../lib/constants/languages";

/**
 * Names a recognition language in the app's language, with the English name as
 * the fallback for a code the browser cannot name ("jw"). "auto" reads "Auto".
 */
export const useLanguageName = (): ((code: string) => string) => {
  const { t, i18n } = useTranslation();
  const displayNames = useMemo(
    () => new Intl.DisplayNames([i18n.language], { type: "language" }),
    [i18n.language],
  );
  return useCallback(
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
};
