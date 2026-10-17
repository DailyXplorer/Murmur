import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface MeetingsToggleProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const MeetingsToggle: React.FC<MeetingsToggleProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    return (
      <ToggleSwitch
        checked={getSetting("meetings_enabled") ?? true}
        onChange={(enabled) => updateSetting("meetings_enabled", enabled)}
        isUpdating={isUpdating("meetings_enabled")}
        label={t("settings.advanced.meetingsToggle.label")}
        description={t("settings.advanced.meetingsToggle.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
