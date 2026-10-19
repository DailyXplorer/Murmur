import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import {
  statusTranslationKey,
  useCodexStatus,
} from "@/components/transcription/providerStatus";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { SettingsPage } from "../../ui/SettingsPage";
import { AiCleanup } from "../AiCleanup";
import { CustomWords } from "../CustomWords";
import { WindowNames } from "../WindowNames";
import { WhisperMode } from "../WhisperMode";
import { AppendTrailingSpace } from "../AppendTrailingSpace";
import { PasteMethodSetting } from "../PasteMethod";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { AutoSubmit } from "../AutoSubmit";

export const TranscriptionSettings: React.FC = () => {
  const { t } = useTranslation();
  const { invalidatePendingRefreshes, refreshStatus, status } =
    useCodexStatus();

  useEffect(() => {
    void refreshStatus();
    const handleFocus = () => void refreshStatus();
    window.addEventListener("focus", handleFocus);
    return () => {
      invalidatePendingRefreshes();
      window.removeEventListener("focus", handleFocus);
    };
  }, [invalidatePendingRefreshes, refreshStatus]);

  return (
    <SettingsPage label={t("sidebar.transcription")}>
      <SettingsGroup title={t("settings.transcription.groups.service")}>
        <SettingContainer
          title={t("onboarding.providerLabels.codex")}
          description={t("settings.transcription.sessionDescription")}
          grouped={true}
          layout="stacked"
        >
          <span className="min-w-0 text-pretty text-sm text-text/80">
            {t(statusTranslationKey(status))}
          </span>
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup title={t("settings.transcription.groups.processing")}>
        <AiCleanup descriptionMode="tooltip" grouped={true} />
        <WindowNames descriptionMode="tooltip" grouped={true} />
        <WhisperMode descriptionMode="tooltip" grouped={true} />
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
