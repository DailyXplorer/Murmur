import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

/** Off by default: a dictation under 20 words skips the default cleanup. */
export const CleanupShortDictations: React.FC<{ noAccount: boolean }> =
  React.memo(({ noAccount }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const off = noAccount || !(getSetting("ai_cleanup_enabled") ?? true);

    return (
      <ToggleSwitch
        checked={getSetting("cleanup_short_dictations") ?? false}
        onChange={(enabled) =>
          updateSetting("cleanup_short_dictations", enabled)
        }
        disabled={off}
        isUpdating={isUpdating("cleanup_short_dictations")}
        label={t("settings.transcription.cleanupShortDictations.title")}
        description={t(
          "settings.transcription.cleanupShortDictations.description",
        )}
        grouped={true}
      />
    );
  });
