import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { ToggleSwitch } from "../ui/ToggleSwitch";

/** Whether this Mac can tell that a call started: not Intel, nor macOS
 * before 14. */
export const useCallDetection = (): boolean => {
  const [supported, setSupported] = useState(false);

  useEffect(() => {
    let current = true;
    void commands.getMeetingCapabilities().then((capabilities) => {
      if (current) setSupported(capabilities.call_detection);
    });
    return () => {
      current = false;
    };
  }, []);

  return supported;
};

/** Whether to offer recording when a call starts. Hidden where the Mac
 * cannot tell. */
export const MeetingCallPrompt: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const supported = useCallDetection();

  if (!supported) return null;
  return (
    <ToggleSwitch
      checked={getSetting("meeting_call_prompt") ?? true}
      onChange={(enabled) => updateSetting("meeting_call_prompt", enabled)}
      isUpdating={isUpdating("meeting_call_prompt")}
      label={t("settings.advanced.meetingCallPrompt.label")}
      description={t("settings.advanced.meetingCallPrompt.description")}
      grouped={true}
    />
  );
};
