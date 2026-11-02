import React from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { ToggleSwitch } from "../ui/ToggleSwitch";

export const MeetingLiveTranscript: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  return (
    <ToggleSwitch
      checked={getSetting("meeting_live_transcript") ?? true}
      onChange={(enabled) => updateSetting("meeting_live_transcript", enabled)}
      isUpdating={isUpdating("meeting_live_transcript")}
      label={t("settings.advanced.meetingLiveTranscript.label")}
      description={t("settings.advanced.meetingLiveTranscript.description")}
      grouped={true}
    />
  );
};
