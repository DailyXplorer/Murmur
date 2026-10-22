import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type AnalysisSupport, type Engine } from "@/bindings";
import { useCodexStatus } from "@/components/transcription/providerStatus";
import { useLocalModel } from "@/hooks/useLocalModel";
import { useSettings } from "@/hooks/useSettings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingsPage } from "../../ui/SettingsPage";
import { RadioOption } from "../../ui/RadioOption";
import { AiCleanup } from "../AiCleanup";
import { DictationStyleSetting } from "../DictationStyle";
import { CustomWords } from "../CustomWords";
import { WindowNames } from "../WindowNames";
import { WhisperMode } from "../WhisperMode";
import { AppendTrailingSpace } from "../AppendTrailingSpace";
import { PasteMethodSetting } from "../PasteMethod";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { AutoSubmit } from "../AutoSubmit";
import { ChatGptRow } from "./ChatGptRow";
import { LocalModelRow } from "./LocalModelRow";

export const TranscriptionSettings: React.FC = () => {
  const { t } = useTranslation();
  const { invalidatePendingRefreshes, refreshStatus, status } =
    useCodexStatus();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const model = useLocalModel();
  const [analysis, setAnalysis] = useState<AnalysisSupport | null>(null);

  useEffect(() => {
    void refreshStatus();
    const handleFocus = () => void refreshStatus();
    window.addEventListener("focus", handleFocus);
    return () => {
      invalidatePendingRefreshes();
      window.removeEventListener("focus", handleFocus);
    };
  }, [invalidatePendingRefreshes, refreshStatus]);

  useEffect(() => {
    let disposed = false;
    void commands.getMeetingCapabilities().then((capabilities) => {
      if (!disposed) setAnalysis(capabilities.analysis);
    });
    return () => {
      disposed = true;
    };
  }, []);

  const engine = getSetting("transcription_engine") ?? "cloud";
  const chooseEngine = async (value: Engine) => {
    if (!(await updateSetting("transcription_engine", value))) {
      toast.error(t("settings.transcription.engine.error"));
    }
  };
  // Unknown while it is checked: nothing is disabled until it is known.
  const noAccount = status.kind === "unavailable";
  const unsupported =
    model?.state.state === "unsupported" ? model.state.support : null;

  return (
    <SettingsPage label={t("sidebar.transcription")}>
      <SettingsGroup title={t("settings.transcription.groups.engine")}>
        <div
          role="radiogroup"
          aria-label={t("settings.transcription.groups.engine")}
          className="divide-y divide-mid-gray/20"
        >
          <RadioOption
            name="transcription-engine"
            value="local"
            title={t("settings.transcription.engine.local.title")}
            checked={engine === "local"}
            disabled={
              model === null ||
              unsupported !== null ||
              isUpdating("transcription_engine")
            }
            description={
              unsupported
                ? t(`settings.transcription.engine.${unsupported}`)
                : t("settings.transcription.engine.local.description")
            }
            onSelect={(value) => void chooseEngine(value)}
          />
          <RadioOption
            name="transcription-engine"
            value="cloud"
            title={t("settings.transcription.engine.cloud.title")}
            checked={engine === "cloud"}
            disabled={isUpdating("transcription_engine")}
            description={t("settings.transcription.engine.cloud.description")}
            onSelect={(value) => void chooseEngine(value)}
          />
        </div>
        {engine === "local" && model && (
          <LocalModelRow
            status={model}
            signedIn={!noAccount}
            meetingsUseIt={analysis === "available"}
          />
        )}
        <ChatGptRow
          status={status}
          refreshStatus={refreshStatus}
          engine={engine}
        />
      </SettingsGroup>

      <SettingsGroup title={t("settings.transcription.groups.processing")}>
        <AiCleanup
          descriptionMode="tooltip"
          grouped={true}
          needsAccount={noAccount}
        />
        <DictationStyleSetting noAccount={noAccount} />
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
