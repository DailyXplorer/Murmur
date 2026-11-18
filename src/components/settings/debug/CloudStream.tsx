import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

interface CloudStreamProps {
  grouped?: boolean;
}

export const CloudStream: React.FC<CloudStreamProps> = ({
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  return (
    <ToggleSwitch
      checked={getSetting("cloud_stream_enabled") ?? true}
      onChange={(enabled) => updateSetting("cloud_stream_enabled", enabled)}
      isUpdating={isUpdating("cloud_stream_enabled")}
      label={t("settings.debug.cloudStream.label")}
      description={t("settings.debug.cloudStream.description")}
      grouped={grouped}
    />
  );
};
