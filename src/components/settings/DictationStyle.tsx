import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { DictationStyle } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";

export const DICTATION_STYLES: DictationStyle[] = [
  "formal",
  "normal",
  "casual",
];

/** A style is applied by AI cleanup, which also needs a ChatGPT account. The
 * AI cleanup row above says when the account is missing. */
export const DictationStyleSetting: React.FC<{ noAccount: boolean }> = ({
  noAccount,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const off = noAccount || !(getSetting("ai_cleanup_enabled") ?? true);

  const choose = async (style: DictationStyle) => {
    if (!(await updateSetting("dictation_style", style))) {
      toast.error(t("settings.transcription.style.error"));
    }
  };

  return (
    <SettingContainer
      title={t("settings.transcription.style.title")}
      description={t("settings.transcription.style.description")}
      grouped={true}
      disabled={off}
      detail={
        off && (
          <p className="text-pretty text-xs text-text/60">
            {t("settings.transcription.style.cleanupOff")}
          </p>
        )
      }
    >
      <Dropdown
        options={DICTATION_STYLES.map((style) => ({
          value: style,
          label: t(`settings.transcription.style.options.${style}`),
        }))}
        selectedValue={getSetting("dictation_style") ?? "normal"}
        onSelect={(style) => void choose(style as DictationStyle)}
        disabled={off || isUpdating("dictation_style")}
      />
    </SettingContainer>
  );
};
