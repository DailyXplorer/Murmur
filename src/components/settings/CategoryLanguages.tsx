import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AppCategory } from "@/bindings";
import { useLanguageName } from "@/hooks/useLanguageName";
import { useSettings } from "@/hooks/useSettings";
import { SELECTABLE_LANGUAGES } from "@/lib/constants/languages";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";
import { KINDS } from "./CategoryStyles";
import { LanguageMenu } from "./LanguageMenu";
import { CATEGORY_LABEL_KEYS } from "./insights/insightsModel";

/** No language code is this long, so it never names one. */
const GENERAL = "general";

/** One dictation language per kind of app, for the kinds that should not
 * follow the language chosen in General: French in messages and English in a
 * code editor, say. Unlike a style, a language needs neither AI cleanup nor an
 * account. */
export const CategoryLanguages: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const languageName = useLanguageName();
  const languages = getSetting("category_languages") ?? {};
  const general = t("settings.transcription.styleByApp.options.general");

  const options = useMemo(
    () => [
      { value: GENERAL, name: general, label: "" },
      ...SELECTABLE_LANGUAGES.map((language) => ({
        ...language,
        name: languageName(language.value),
      })),
    ],
    [general, languageName],
  );

  const choose = async (category: AppCategory, value: string) => {
    const next = { ...languages };
    if (value === GENERAL) delete next[category];
    else next[category] = value;
    if (!(await updateSetting("category_languages", next))) {
      toast.error(t("settings.transcription.languageByApp.error"));
    }
  };

  return (
    <SettingsGroup title={t("settings.transcription.groups.languageByApp")}>
      {KINDS.map((category) => {
        const language = languages[category];
        return (
          <div key={category} data-testid={`language-${category}`}>
            <SettingContainer
              title={t(CATEGORY_LABEL_KEYS[category])}
              description={t(
                "settings.transcription.languageByApp.description",
              )}
              grouped={true}
            >
              <div className="flex min-w-0 items-center">
                <LanguageMenu
                  options={options}
                  selected={language ?? GENERAL}
                  selectedName={language ? languageName(language) : general}
                  onSelect={(value) => choose(category, value)}
                  disabled={isUpdating("category_languages")}
                />
              </div>
            </SettingContainer>
          </div>
        );
      })}
    </SettingsGroup>
  );
};
