import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface WindowNamesProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const WindowNames: React.FC<WindowNamesProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    return (
      <ToggleSwitch
        checked={getSetting("window_names_enabled") ?? true}
        onChange={(enabled) => updateSetting("window_names_enabled", enabled)}
        isUpdating={isUpdating("window_names_enabled")}
        label={t("settings.transcription.windowNames.title")}
        description={t("settings.transcription.windowNames.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
