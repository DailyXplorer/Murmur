import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface LeadingSpaceProps {
  grouped?: boolean;
}

export const LeadingSpace: React.FC<LeadingSpaceProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    return (
      <ToggleSwitch
        checked={getSetting("leading_space") ?? true}
        onChange={(enabled) => updateSetting("leading_space", enabled)}
        isUpdating={isUpdating("leading_space")}
        label={t("settings.debug.leadingSpace.label")}
        description={t("settings.debug.leadingSpace.description")}
        grouped={grouped}
      />
    );
  },
);
