import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface AiCleanupProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  /** Cleanup runs through ChatGPT, so it waits for an account. */
  needsAccount?: boolean;
}

export const AiCleanup: React.FC<AiCleanupProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false, needsAccount = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const enabled = getSetting("ai_cleanup_enabled") ?? true;

    return (
      <ToggleSwitch
        checked={enabled && !needsAccount}
        onChange={(value) => updateSetting("ai_cleanup_enabled", value)}
        disabled={needsAccount}
        isUpdating={isUpdating("ai_cleanup_enabled")}
        label={t("settings.transcription.aiCleanup.title")}
        description={t("settings.transcription.aiCleanup.description")}
        detail={
          needsAccount && (
            <p className="text-pretty text-xs text-text/60">
              {t("settings.transcription.aiCleanup.needsAccount")}
            </p>
          )
        }
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
