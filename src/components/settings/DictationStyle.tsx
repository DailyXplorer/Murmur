import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { DictationStyle } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { RadioOption } from "../ui/RadioOption";
import { SettingContainer } from "../ui/SettingContainer";

const STYLES: DictationStyle[] = ["formal", "normal", "casual"];

/** A style is applied by AI cleanup, which runs through ChatGPT. */
type StyleAvailability = "available" | "cleanupOff" | "needsAccount";

const styleAvailability = ({
  cleanupEnabled,
  noAccount,
}: {
  cleanupEnabled: boolean;
  noAccount: boolean;
}): StyleAvailability =>
  noAccount ? "needsAccount" : cleanupEnabled ? "available" : "cleanupOff";

interface DictationStyleOptionsProps {
  value: DictationStyle;
  disabled: boolean;
  onSelect: (style: DictationStyle) => void;
  className?: string;
  optionClassName?: string;
}

/** The three styles, each with the same sentence written in it. */
export const DictationStyleOptions: React.FC<DictationStyleOptionsProps> = ({
  value,
  disabled,
  onSelect,
  className,
  optionClassName,
}) => {
  const { t } = useTranslation();
  return (
    <div
      role="radiogroup"
      aria-label={t("settings.transcription.style.title")}
      className={className}
    >
      {STYLES.map((style) => (
        <RadioOption
          key={style}
          name="dictation-style"
          value={style}
          checked={value === style}
          disabled={disabled}
          title={t(`settings.transcription.style.${style}.title`)}
          description={t(`settings.transcription.style.${style}.example`)}
          onSelect={onSelect}
          className={optionClassName}
        />
      ))}
    </div>
  );
};

export const DictationStyleSetting: React.FC<{ noAccount: boolean }> = ({
  noAccount,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const availability = styleAvailability({
    cleanupEnabled: getSetting("ai_cleanup_enabled") ?? true,
    noAccount,
  });

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
      layout="stacked"
      disabled={availability !== "available"}
      detail={
        availability !== "available" && (
          <p className="-mt-1.5 mb-1 text-pretty text-xs text-text/60">
            {t(`settings.transcription.style.${availability}`)}
          </p>
        )
      }
    >
      <DictationStyleOptions
        value={getSetting("dictation_style") ?? "normal"}
        disabled={availability !== "available" || isUpdating("dictation_style")}
        onSelect={(style) => void choose(style)}
        optionClassName="py-1.5"
      />
    </SettingContainer>
  );
};
