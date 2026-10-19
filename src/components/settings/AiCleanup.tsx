import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface AiCleanupProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const AiCleanup: React.FC<AiCleanupProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const enabled = getSetting("ai_cleanup_enabled") ?? true;

    return (
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("ai_cleanup_enabled", value)}
        isUpdating={isUpdating("ai_cleanup_enabled")}
        label={t("settings.transcription.aiCleanup.title")}
        description={t("settings.transcription.aiCleanup.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
