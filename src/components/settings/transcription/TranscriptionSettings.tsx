import React, { useCallback, useEffect, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { commands, type TranscriptionProvider } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import {
  isConfiguredProvider,
  type ProviderStatus,
  useProviderStatuses,
} from "@/components/transcription/providerStatus";
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

const statusTranslationKey = (status: ProviderStatus): string => {
  switch (status.kind) {
    case "checking":
      return "settings.transcription.checking";
    case "configured":
      return "onboarding.provider.configured";
    case "unavailable":
      return status.reason === "notInstalled"
        ? "settings.transcription.notInstalled"
        : "onboarding.providerLabels.unavailable";
    case "error":
      return "settings.transcription.statusUnavailable";
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
};

export const TranscriptionSettings: React.FC = () => {
  const { t } = useTranslation();
  const { settings, updateSetting, isUpdating } = useSettings();
  const { invalidatePendingRefreshes, refreshStatuses, statuses } =
    useProviderStatuses();

  useEffect(() => {
    void refreshStatuses();
    const handleFocus = () => void refreshStatuses();
    window.addEventListener("focus", handleFocus);
    return () => {
      invalidatePendingRefreshes();
      window.removeEventListener("focus", handleFocus);
    };
  }, [invalidatePendingRefreshes, refreshStatuses]);

  const provider = settings?.transcription_provider ?? "codex";
  const providerOptions = useMemo(
    () => [
      {
        value: "codex",
        label: t("onboarding.providerLabels.codex"),
        disabled: !isConfiguredProvider(statuses.codex),
      },
      {
        value: "gemini",
        label: t("onboarding.providerLabels.antigravity"),
        disabled: !isConfiguredProvider(statuses.gemini),
      },
    ],
    [statuses.codex, statuses.gemini, t],
  );

  const changeProvider = useCallback(
    async (value: string) => {
      if (value !== "codex" && value !== "gemini") return;
      const selectedProvider: TranscriptionProvider = value;
      const updated = await updateSetting(
        "transcription_provider",
        selectedProvider,
      );
      if (!updated) {
        toast.error(t("onboarding.provider.providerChangeFailed"));
      }
    },
    [t, updateSetting],
  );

  const openAntigravity = useCallback(async () => {
    try {
      const result = await commands.openAntigravity();
      if (result.status === "error") throw new Error(result.error);
    } catch (error) {
      console.warn("Failed to open Antigravity:", error);
      toast.error(t("onboarding.provider.openFailed"));
    }
  }, [t]);

  const installAntigravity = useCallback(async () => {
    try {
      await openUrl("https://antigravity.google/");
    } catch (error) {
      console.warn("Failed to open the Antigravity download page:", error);
      toast.error(t("onboarding.provider.installFailed"));
    }
  }, [t]);

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
            onSelect={(value) => void changeProvider(value)}
            disabled={isUpdating("transcription_provider")}
          />
        </SettingContainer>

        <SettingContainer
          title={t("onboarding.providerLabels.codex")}
          description={t("settings.transcription.sessionDescription")}
          grouped={true}
        >
          <span className="text-sm text-text/80">
            {t(statusTranslationKey(statuses.codex))}
          </span>
        </SettingContainer>

        <SettingContainer
          title={t("onboarding.providerLabels.antigravity")}
          description={t("onboarding.provider.antigravityDescription")}
          grouped={true}
        >
          <div className="flex min-w-0 items-center gap-2">
            <span className="rounded bg-logo-primary/20 px-1.5 py-0.5 text-xs font-medium text-logo-primary">
              {t("onboarding.provider.experimental")}
            </span>
            <span className="min-w-0 flex-1 truncate text-end text-sm text-text/80">
              {t(statusTranslationKey(statuses.gemini))}
            </span>
            {statuses.gemini.kind === "unavailable" &&
              statuses.gemini.reason === "notInstalled" && (
                <Button
                  size="sm"
                  variant="secondary"
                  className="shrink-0"
                  onClick={() => void installAntigravity()}
                >
                  {t("settings.transcription.installAntigravity")}
                </Button>
              )}
            {statuses.gemini.kind === "unavailable" &&
              statuses.gemini.reason === "notSignedIn" && (
                <Button
                  size="sm"
                  variant="secondary"
                  className="shrink-0"
                  onClick={() => void openAntigravity()}
                >
                  {t("settings.transcription.openAntigravity")}
                </Button>
              )}
          </div>
        </SettingContainer>
        <div className="flex justify-end px-4 pb-2">
          <Button
            size="sm"
            variant="secondary"
            onClick={() => void refreshStatuses()}
          >
            {t("onboarding.provider.retry")}
          </Button>
        </div>
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
