import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type AppleIntelligenceStatus,
  type CleanupEngine,
} from "@/bindings";
import { localizedLanguageName } from "@/lib/constants/languages";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface AiCleanupProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

/**
 * Re-read on focus, since the user may have just turned it on in System
 * Settings, and when the dictation language changes.
 */
const useAppleIntelligenceStatus = (languageKey: string) => {
  const [status, setStatus] = useState<AppleIntelligenceStatus | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = () =>
      commands
        .getAppleIntelligenceStatus()
        .then((next) => {
          if (active) setStatus(next);
        })
        .catch((error) =>
          console.warn("Failed to read Apple Intelligence status:", error),
        );
    void refresh();
    window.addEventListener("focus", refresh);
    return () => {
      active = false;
      window.removeEventListener("focus", refresh);
    };
  }, [languageKey]);

  return status;
};

export const AiCleanup: React.FC<AiCleanupProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t, i18n } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const language = getSetting("selected_language") ?? "auto";
    const provider = getSetting("transcription_provider");
    const appleStatus = useAppleIntelligenceStatus(`${provider}:${language}`);
    const enabled = getSetting("ai_cleanup_enabled") ?? true;
    const engine: CleanupEngine = getSetting("cleanup_engine") ?? "apple";

    return (
      <>
        <ToggleSwitch
          checked={enabled}
          onChange={(value) => updateSetting("ai_cleanup_enabled", value)}
          isUpdating={isUpdating("ai_cleanup_enabled")}
          label={t("settings.transcription.aiCleanup.title")}
          description={t("settings.transcription.aiCleanup.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        />
        <SettingContainer
          title={t("settings.transcription.cleanupEngine.title")}
          description={t("settings.transcription.cleanupEngine.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
          disabled={!enabled}
        >
          <Dropdown
            options={[
              {
                value: "apple",
                label: t("settings.transcription.cleanupEngine.options.apple"),
              },
              {
                value: "codex",
                label: t("settings.transcription.cleanupEngine.options.codex"),
              },
            ]}
            selectedValue={engine}
            onSelect={(value) =>
              updateSetting("cleanup_engine", value as CleanupEngine)
            }
            disabled={!enabled || isUpdating("cleanup_engine")}
          />
        </SettingContainer>
        {enabled && engine === "apple" && appleStatus !== null && (
          <SettingContainer
            title={t("settings.transcription.cleanupEngine.status.title")}
            description={t(
              "settings.transcription.cleanupEngine.status.description",
            )}
            descriptionMode={descriptionMode}
            grouped={grouped}
          >
            <span
              className="text-end text-sm text-text/80"
              data-testid="apple-intelligence-status"
            >
              {t(`settings.transcription.cleanupEngine.status.${appleStatus}`, {
                language: localizedLanguageName(language, i18n.language),
              })}
            </span>
          </SettingContainer>
        )}
      </>
    );
  },
);
