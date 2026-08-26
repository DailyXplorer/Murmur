import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { type as osType } from "@tauri-apps/plugin-os";
import { openUrl } from "@tauri-apps/plugin-opener";
import { commands } from "@/bindings";
import type {
  CodexAuthStatus,
  GeminiStatus,
  TranscriptionProvider,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dropdown } from "../../ui/Dropdown";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";

const EMPTY_CODEX_STATUS: CodexAuthStatus = { signed_in: false };
const EMPTY_GEMINI_STATUS: GeminiStatus = {
  available_on_platform: false,
  installed: false,
  signed_in: false,
};

export const TranscriptionSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, updateSetting, isUpdating } = useSettings();
  const isMacOS = osType() === "macos";
  const [codexStatus, setCodexStatus] = useState<CodexAuthStatus | null>(null);
  const [geminiStatus, setGeminiStatus] = useState<GeminiStatus | null>(null);

  const refreshStatuses = useCallback(async () => {
    const [codex, gemini] = await Promise.allSettled([
      commands.getCodexAuthStatus(),
      commands.getGeminiStatus(),
    ]);
    setCodexStatus(
      codex.status === "fulfilled" ? codex.value : EMPTY_CODEX_STATUS,
    );
    setGeminiStatus(
      gemini.status === "fulfilled" ? gemini.value : EMPTY_GEMINI_STATUS,
    );
  }, []);

  useEffect(() => {
    void refreshStatuses();
    const handleFocus = () => void refreshStatuses();
    window.addEventListener("focus", handleFocus);
    return () => window.removeEventListener("focus", handleFocus);
  }, [refreshStatuses]);

  const provider = settings?.transcription_provider ?? "codex";
  const providerOptions = useMemo(
    () => [
      { value: "codex", label: t("settings.transcription.codex") },
      {
        value: "gemini",
        label: t("settings.transcription.gemini"),
        disabled: !geminiStatus?.installed,
      },
    ],
    [geminiStatus?.installed, t],
  );

  const sessionLabel = (signedIn: boolean | null | undefined) => {
    if (signedIn == null) return t("settings.transcription.checking");
    return signedIn
      ? t("settings.transcription.ready")
      : t("settings.transcription.missing");
  };

  const openAntigravity = async () => {
    const result = await commands.openAntigravity();
    if (result.status === "error") console.error(result.error);
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.transcription.title")}>
        {isMacOS && (
          <SettingContainer
            title={t("settings.transcription.providerTitle")}
            description={t("settings.transcription.providerDescription")}
            grouped={true}
          >
            <Dropdown
              options={providerOptions}
              selectedValue={provider}
              onSelect={(value) =>
                void updateSetting(
                  "transcription_provider",
                  value as TranscriptionProvider,
                )
              }
              disabled={isUpdating("transcription_provider")}
            />
          </SettingContainer>
        )}

        <SettingContainer
          title={t("settings.transcription.codex")}
          description={t("settings.transcription.sessionDescription")}
          grouped={true}
        >
          <span className="text-sm text-text/80">
            {sessionLabel(codexStatus?.signed_in)}
          </span>
        </SettingContainer>

        {isMacOS && (
          <SettingContainer
            title={t("settings.transcription.gemini")}
            description={t("settings.transcription.geminiDescription")}
            grouped={true}
          >
            <div className="flex items-center gap-2">
              <span className="text-sm text-text/80">
                {geminiStatus == null
                  ? t("settings.transcription.checking")
                  : !geminiStatus.installed
                    ? t("settings.transcription.notInstalled")
                    : sessionLabel(geminiStatus.signed_in)}
              </span>
              {geminiStatus && !geminiStatus.installed && (
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void openUrl("https://antigravity.google/")}
                >
                  {t("settings.transcription.installAntigravity")}
                </Button>
              )}
              {geminiStatus?.installed && !geminiStatus.signed_in && (
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void openAntigravity()}
                >
                  {t("settings.transcription.openAntigravity")}
                </Button>
              )}
            </div>
          </SettingContainer>
        )}
      </SettingsGroup>
    </div>
  );
};
