import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { commands } from "@/bindings";
import type { CodexAuthStatus, GeminiStatus } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dropdown } from "../../ui/Dropdown";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { SettingsPage } from "../../ui/SettingsPage";
import { FillerWordRemoval } from "../FillerWordRemoval";
import { CustomWords } from "../CustomWords";
import { AppendTrailingSpace } from "../AppendTrailingSpace";
import { PasteMethodSetting } from "../PasteMethod";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { AutoSubmit } from "../AutoSubmit";

const EMPTY_CODEX_STATUS: CodexAuthStatus = { signed_in: false };
export const TranscriptionSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, updateSetting, isUpdating } = useSettings();
  const [codexStatus, setCodexStatus] = useState<CodexAuthStatus | null>(null);
  const [geminiStatus, setGeminiStatus] = useState<GeminiStatus | null>(null);
  const [geminiStatusError, setGeminiStatusError] = useState(false);

  const refreshStatuses = useCallback(async () => {
    const [codex, gemini] = await Promise.allSettled([
      commands.getCodexAuthStatus(),
      commands.getGeminiStatus(),
    ]);
    setCodexStatus(
      codex.status === "fulfilled" ? codex.value : EMPTY_CODEX_STATUS,
    );
    if (gemini.status === "fulfilled") {
      setGeminiStatus(gemini.value);
      setGeminiStatusError(false);
    } else {
      setGeminiStatusError(true);
    }
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
        disabled:
          geminiStatusError ||
          !(geminiStatus?.installed && geminiStatus.signed_in),
      },
    ],
    [geminiStatus?.installed, geminiStatus?.signed_in, geminiStatusError, t],
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
    <SettingsPage label={t("sidebar.transcription")}>
      <SettingsGroup title={t("settings.transcription.groups.service")}>
        <SettingContainer
          title={t("settings.transcription.providerTitle")}
          description={t("settings.transcription.providerDescription")}
          grouped={true}
        >
          <Dropdown
            options={providerOptions}
            selectedValue={provider}
            className="w-[260px]"
            onSelect={(value) => {
              if (value !== "codex" && value !== "gemini") return;
              void updateSetting("transcription_provider", value);
            }}
            disabled={isUpdating("transcription_provider")}
          />
        </SettingContainer>

        <SettingContainer
          title={t("settings.transcription.codex")}
          description={t("settings.transcription.sessionDescription")}
          grouped={true}
        >
          <span className="text-sm text-text/80">
            {sessionLabel(codexStatus?.signed_in)}
          </span>
        </SettingContainer>

        <SettingContainer
          title={t("settings.transcription.gemini")}
          description={t("settings.transcription.geminiDescription")}
          grouped={true}
        >
          <div className="flex items-center gap-2">
            <span className="text-sm text-text/80">
              {geminiStatusError
                ? t("settings.transcription.statusUnavailable")
                : geminiStatus == null
                  ? t("settings.transcription.checking")
                  : !geminiStatus.installed
                    ? t("settings.transcription.notInstalled")
                    : sessionLabel(geminiStatus.signed_in)}
            </span>
            {!geminiStatusError && geminiStatus && !geminiStatus.installed && (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void openUrl("https://antigravity.google/")}
              >
                {t("settings.transcription.installAntigravity")}
              </Button>
            )}
            {!geminiStatusError &&
              geminiStatus?.installed &&
              !geminiStatus.signed_in && (
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
      </SettingsGroup>

      <SettingsGroup title={t("settings.transcription.groups.processing")}>
        <FillerWordRemoval descriptionMode="tooltip" grouped={true} />
        <CustomWords descriptionMode="tooltip" grouped={true} />
        <AppendTrailingSpace descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.transcription.groups.output")}>
        <PasteMethodSetting descriptionMode="tooltip" grouped={true} />
        <ClipboardHandlingSetting descriptionMode="tooltip" grouped={true} />
        <AutoSubmit descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    </SettingsPage>
  );
};
