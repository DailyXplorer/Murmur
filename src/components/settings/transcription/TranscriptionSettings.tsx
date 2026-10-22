import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { commands, type AnalysisSupport, type Engine } from "@/bindings";
import {
  statusTranslationKey,
  useCodexStatus,
} from "@/components/transcription/providerStatus";
import { useLocalModel } from "@/hooks/useLocalModel";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "../../ui/Button";
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
import { LocalModelRow } from "./LocalModelRow";

const EngineOption: React.FC<{
  engine: Engine;
  checked: boolean;
  disabled: boolean;
  description: string;
  onSelect: (engine: Engine) => void;
}> = ({ engine, checked, disabled, description, onSelect }) => {
  const { t } = useTranslation();
  return (
    <label
      className={`flex items-start gap-3 px-4 py-2.5 ${disabled ? "cursor-not-allowed" : "cursor-pointer"}`}
    >
      <input
        type="radio"
        name="transcription-engine"
        value={engine}
        checked={checked}
        disabled={disabled}
        onChange={() => onSelect(engine)}
        className="mt-0.5 size-4 shrink-0 accent-logo-primary disabled:opacity-50 dark:[color-scheme:dark]"
      />
      <span className={`min-w-0 ${disabled ? "opacity-50" : ""}`}>
        <span className="block text-sm font-medium">
          {t(`settings.transcription.engine.${engine}.title`)}
        </span>
        <span className="block text-pretty text-sm text-text/70">
          {description}
        </span>
      </span>
    </label>
  );
};

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
          <EngineOption
            engine="local"
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
          <EngineOption
            engine="cloud"
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
        {engine === "local" && noAccount && (
          <SettingContainer
            title={t("settings.transcription.chatgpt.title")}
            description={t("settings.transcription.chatgpt.optional")}
            descriptionMode="inline"
            grouped={true}
          >
            <Button
              onClick={() =>
                void openUrl("https://openai.com/codex/").catch(() =>
                  toast.error(t("onboarding.provider.codexSetupFailed")),
                )
              }
            >
              {t("onboarding.provider.setupCodex")}
            </Button>
          </SettingContainer>
        )}
        {engine === "cloud" && (
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
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.transcription.groups.processing")}>
        <AiCleanup
          descriptionMode="tooltip"
          grouped={true}
          needsAccount={noAccount}
        />
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
