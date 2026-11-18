import React, { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AppCategory, CategoryStyle } from "@/bindings";
import { useLanguageName } from "@/hooks/useLanguageName";
import { useSettings } from "@/hooks/useSettings";
import { SELECTABLE_LANGUAGES } from "@/lib/constants/languages";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";
import { DICTATION_STYLES } from "./DictationStyle";
import { LanguageMenu } from "./LanguageMenu";
import { CATEGORY_LABEL_KEYS } from "./insights/insightsModel";

/** "Other" has no row: an app Murmur cannot place follows the general style
 * and language. */
const KINDS: AppCategory[] = [
  "ai_prompts",
  "personal_messages",
  "work_messages",
  "emails",
  "documents",
  "coding",
];

/** Stands for "follows the general setting": a kind on it has no entry. No
 * style and no language code is spelled this way. */
const GENERAL = "general";

/**
 * The style and the dictation language of each kind of app, for the kinds
 * that should not follow the general ones. The page shows one row; the six
 * kinds are in its dialog, where each choice is saved as it is made. A style
 * needs AI cleanup and an account, a language needs neither.
 */
export const ByAppType: React.FC<{ noAccount: boolean }> = ({ noAccount }) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const languageName = useLanguageName();
  const [open, setOpen] = useState(false);
  const stylesOff = noAccount || !(getSetting("ai_cleanup_enabled") ?? true);
  const styles = getSetting("category_styles") ?? {};
  const languages = getSetting("category_languages") ?? {};
  const customized = KINDS.filter(
    (kind) => styles[kind] !== undefined || languages[kind] !== undefined,
  ).length;
  const general = t("settings.transcription.styleByApp.options.general");

  const styleOptions = [
    { value: GENERAL, label: general },
    ...DICTATION_STYLES.map((style) => ({
      value: style,
      label: t(`settings.transcription.style.options.${style}`),
    })),
    { value: "off", label: t("settings.transcription.styleByApp.options.off") },
  ];
  const languageOptions = useMemo(
    () => [
      { value: GENERAL, name: general, label: "" },
      ...SELECTABLE_LANGUAGES.map((language) => ({
        ...language,
        name: languageName(language.value),
      })),
    ],
    [general, languageName],
  );

  const chooseStyle = async (kind: AppCategory, value: string) => {
    const next = { ...styles };
    if (value === GENERAL) delete next[kind];
    else next[kind] = value as CategoryStyle;
    if (!(await updateSetting("category_styles", next))) {
      toast.error(t("settings.transcription.style.error"));
    }
  };

  const chooseLanguage = async (kind: AppCategory, value: string) => {
    const next = { ...languages };
    if (value === GENERAL) delete next[kind];
    else next[kind] = value;
    if (!(await updateSetting("category_languages", next))) {
      toast.error(t("settings.transcription.languageByApp.error"));
    }
  };

  const styleColumn = t("settings.transcription.byApp.columns.style");
  const languageColumn = t("settings.transcription.byApp.columns.language");

  return (
    <SettingsGroup title={t("settings.transcription.groups.byApp")}>
      <SettingContainer
        title={t("settings.transcription.byApp.title")}
        description={t("settings.transcription.byApp.description")}
        grouped={true}
        detail={
          customized > 0 && (
            <p data-live-status className="text-pretty text-sm text-text/70">
              {t("settings.transcription.byApp.customized", {
                count: customized,
              })}
            </p>
          )
        }
      >
        <Button onClick={() => setOpen(true)}>
          {t("settings.transcription.byApp.customize")}
        </Button>
      </SettingContainer>

      <Dialog
        open={open}
        title={t("settings.transcription.byApp.dialogTitle")}
        titleInfo={
          stylesOff ? t("settings.transcription.style.cleanupOff") : undefined
        }
        description={t("settings.transcription.byApp.description")}
        closeLabel={t("common.close")}
        onOpenChange={setOpen}
        wide={true}
      >
        <div className="grid grid-cols-[minmax(0,1fr)_200px_200px] items-center gap-x-3 gap-y-2">
          <span className="text-xs font-medium text-text/60">
            {t("settings.transcription.byApp.columns.kind")}
          </span>
          <span className="text-xs font-medium text-text/60">
            {styleColumn}
          </span>
          <span className="text-xs font-medium text-text/60">
            {languageColumn}
          </span>
          {KINDS.map((kind) => {
            const kindLabel = t(CATEGORY_LABEL_KEYS[kind]);
            const language = languages[kind];
            return (
              <React.Fragment key={kind}>
                <span
                  className="min-w-0 truncate text-sm font-medium"
                  title={kindLabel}
                >
                  {kindLabel}
                </span>
                <div
                  role="group"
                  aria-label={`${kindLabel}: ${styleColumn}`}
                  data-testid={`style-${kind}`}
                >
                  <Dropdown
                    options={styleOptions}
                    selectedValue={styles[kind] ?? GENERAL}
                    onSelect={(value) => void chooseStyle(kind, value)}
                    disabled={stylesOff || isUpdating("category_styles")}
                  />
                </div>
                <div
                  role="group"
                  aria-label={`${kindLabel}: ${languageColumn}`}
                  data-testid={`language-${kind}`}
                >
                  <LanguageMenu
                    options={languageOptions}
                    selected={language ?? GENERAL}
                    selectedName={language ? languageName(language) : general}
                    onSelect={(value) => chooseLanguage(kind, value)}
                    disabled={isUpdating("category_languages")}
                  />
                </div>
              </React.Fragment>
            );
          })}
        </div>
      </Dialog>
    </SettingsGroup>
  );
};
