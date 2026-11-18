import React from "react";
import { useTranslation } from "react-i18next";
import { isWindows } from "@/lib/platform";
import { useSettings } from "../../hooks/useSettings";
import { ToggleSwitch } from "../ui/ToggleSwitch";

export const MeetingOnDevice: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  return (
    <ToggleSwitch
      checked={getSetting("meeting_on_device") ?? false}
      onChange={(enabled) => updateSetting("meeting_on_device", enabled)}
      isUpdating={isUpdating("meeting_on_device")}
      label={t("settings.advanced.meetingOnDevice.label")}
      description={t(
        isWindows
          ? "settings.advanced.meetingOnDevice.descriptionWindows"
          : "settings.advanced.meetingOnDevice.description",
      )}
      grouped={true}
    />
  );
};
