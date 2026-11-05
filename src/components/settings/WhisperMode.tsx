import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface WhisperModeProps {
  grouped?: boolean;
}

export const WhisperMode: React.FC<WhisperModeProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    return (
      <ToggleSwitch
        checked={getSetting("whisper_mode") ?? false}
        onChange={(enabled) => updateSetting("whisper_mode", enabled)}
        isUpdating={isUpdating("whisper_mode")}
        label={t("settings.transcription.whisperMode.title")}
        description={t("settings.transcription.whisperMode.description")}
        grouped={grouped}
      />
    );
  },
);
