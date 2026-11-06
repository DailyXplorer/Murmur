import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { isWindows } from "@/lib/platform";
import { useSettings } from "../../hooks/useSettings";
import { ToggleSwitch } from "../ui/ToggleSwitch";

/** Whether to offer recording when a call starts. Hidden where the computer
 * cannot tell (an Intel Mac, or macOS before 14). FaceTime has no Windows
 * version, so the Windows text does not list it. */
export const MeetingCallPrompt: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
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

  if (!supported) return null;
  return (
    <ToggleSwitch
      checked={getSetting("meeting_call_prompt") ?? true}
      onChange={(enabled) => updateSetting("meeting_call_prompt", enabled)}
      isUpdating={isUpdating("meeting_call_prompt")}
      label={t("settings.advanced.meetingCallPrompt.label")}
      description={t(
        isWindows
          ? "settings.advanced.meetingCallPrompt.descriptionWindows"
          : "settings.advanced.meetingCallPrompt.description",
      )}
      grouped={true}
    />
  );
};
