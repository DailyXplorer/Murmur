import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface DictationLiveTextProps {
  grouped?: boolean;
}

export const DictationLiveText: React.FC<DictationLiveTextProps> = React.memo(
  ({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    // The words come from ChatGPT's stream; the local model gives its text
    // only once the recording has stopped.
    const local = getSetting("transcription_engine") === "local";

    return (
      <ToggleSwitch
        checked={(getSetting("dictation_live_text") ?? false) && !local}
        onChange={(enabled) => updateSetting("dictation_live_text", enabled)}
        disabled={local}
        isUpdating={isUpdating("dictation_live_text")}
        label={t("settings.advanced.overlay.liveText.title")}
        description={[
          local && t("settings.advanced.overlay.liveText.needsChatGpt"),
          t("settings.advanced.overlay.liveText.description"),
        ]
          .filter(Boolean)
          .join(" ")}
        grouped={grouped}
      />
    );
  },
);
