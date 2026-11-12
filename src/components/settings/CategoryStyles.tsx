import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AppCategory, CategoryStyle } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";
import { DICTATION_STYLES } from "./DictationStyle";
import { CATEGORY_LABEL_KEYS } from "./insights/insightsModel";

/** "Other" has no row: an app Murmur cannot place follows the general style. */
const KINDS: AppCategory[] = [
  "ai_prompts",
  "personal_messages",
  "work_messages",
  "emails",
  "documents",
  "coding",
];

const GENERAL = "general";

/** One style per kind of app, for the kinds that should not follow the
 * general dictation style. Like it, a style needs AI cleanup and an account. */
export const CategoryStyles: React.FC<{ noAccount: boolean }> = ({
  noAccount,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const off = noAccount || !(getSetting("ai_cleanup_enabled") ?? true);
  const styles = getSetting("category_styles") ?? {};

  const choose = async (category: AppCategory, value: string) => {
    const next = { ...styles };
    if (value === GENERAL) delete next[category];
    else next[category] = value as CategoryStyle;
    if (!(await updateSetting("category_styles", next))) {
      toast.error(t("settings.transcription.style.error"));
    }
  };

  const options = [
    {
      value: GENERAL,
      label: t("settings.transcription.styleByApp.options.general"),
    },
    ...DICTATION_STYLES.map((style) => ({
      value: style,
      label: t(`settings.transcription.style.options.${style}`),
    })),
    { value: "off", label: t("settings.transcription.styleByApp.options.off") },
  ];
  const description = [
    off && t("settings.transcription.style.cleanupOff"),
    t("settings.transcription.styleByApp.description"),
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <SettingsGroup title={t("settings.transcription.groups.styleByApp")}>
      {KINDS.map((category) => (
        <div key={category} data-testid={`style-${category}`}>
          <SettingContainer
            title={t(CATEGORY_LABEL_KEYS[category])}
            description={description}
            grouped={true}
            disabled={off}
          >
            <Dropdown
              options={options}
              selectedValue={styles[category] ?? GENERAL}
              onSelect={(value) => void choose(category, value)}
              disabled={off || isUpdating("category_styles")}
            />
          </SettingContainer>
        </div>
      ))}
    </SettingsGroup>
  );
};
