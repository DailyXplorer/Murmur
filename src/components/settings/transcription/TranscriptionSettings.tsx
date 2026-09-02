import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { commands } from "@/bindings";
import type {
  CodexAuthStatus,
  GeminiStatus,
  MetaApiStatus,
  MetaAppStatus,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "../../ui/Button";
import { Dropdown } from "../../ui/Dropdown";
import { Input } from "../../ui/Input";
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
  const [metaStatus, setMetaStatus] = useState<MetaApiStatus | null>(null);
  const [metaAppStatus, setMetaAppStatus] = useState<MetaAppStatus | null>(
    null,
  );
  const [metaApiKey, setMetaApiKey] = useState("");
  const [isUpdatingMetaKey, setIsUpdatingMetaKey] = useState(false);

  const refreshStatuses = useCallback(async () => {
    const [codex, gemini, meta, metaApp] = await Promise.allSettled([
      commands.getCodexAuthStatus(),
      commands.getGeminiStatus(),
      commands.getMetaApiStatus(),
      commands.getMetaAppStatus(),
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
    setMetaStatus(
      meta.status === "fulfilled" ? meta.value : { configured: false },
    );
    setMetaAppStatus(
      metaApp.status === "fulfilled"
        ? metaApp.value
        : {
            installed: false,
            dictation_enabled: false,
            hold_fn_enabled: false,
            accessibility_trusted: false,
            runtime_state: "not_running",
            ready: false,
          },
    );
  }, []);

  const metaAppSessionLabel = (status: MetaAppStatus | null) => {
    if (status == null) return t("settings.transcription.checking");
    if (!status.installed)
      return t("settings.transcription.metaAppNotInstalled");
    if (!status.accessibility_trusted)
      return t("onboarding.permissions.accessibility.waiting");
    if (!status.dictation_enabled || !status.hold_fn_enabled)
      return t("settings.transcription.metaAppNeedsSetup");

    const runtimeState = status.runtime_state;
    switch (runtimeState) {
      case "not_running":
        return t("settings.transcription.metaAppNotRunning");
      case "active":
      case "window_visible":
        return t("settings.transcription.metaAppWindowOpen");
      case "dictating":
        return t("settings.transcription.metaAppAlreadyDictating");
      case "inspection_unavailable":
        return t("settings.transcription.statusUnavailable");
      case "ready":
        return t("settings.transcription.ready");
      default: {
        const exhaustive: never = runtimeState;
        return exhaustive;
      }
    }
  };

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
      {
        value: "meta",
        label: t("settings.transcription.meta"),
        disabled: !metaStatus?.configured,
      },
      {
        value: "meta_app",
        label: t("settings.transcription.metaApp"),
        disabled: !metaAppStatus?.ready,
      },
    ],
    [
      geminiStatus?.installed,
      geminiStatus?.signed_in,
      geminiStatusError,
      metaStatus?.configured,
      metaAppStatus?.ready,
      t,
    ],
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

  const openMetaAi = async () => {
    try {
      const result = await commands.openMetaAi();
      if (result.status === "ok") return;
      console.error(result.error);
    } catch (error) {
      console.error(error);
    }
    toast.error(t("errors.transcriptionFailedTitle"), {
      description: t("errors.metaApp.openFailed"),
    });
  };

  const saveMetaApiKey = async () => {
    const apiKey = metaApiKey.trim();
    if (!apiKey) return;

    setIsUpdatingMetaKey(true);
    try {
      const result = await commands.saveMetaApiKey(apiKey);
      if (result.status === "error") {
        toast.error(t("errors.transcriptionFailedTitle"), {
          description: t("errors.metaApi.saveFailed"),
        });
        return;
      }
      setMetaApiKey("");
      await refreshStatuses();
    } finally {
      setIsUpdatingMetaKey(false);
    }
  };

  const clearMetaApiKey = async () => {
    setIsUpdatingMetaKey(true);
    try {
      const result = await commands.clearMetaApiKey();
      if (result.status === "error") {
        toast.error(t("errors.transcriptionFailedTitle"), {
          description: t("errors.metaApi.removeFailed"),
        });
        return;
      }
      await refreshStatuses();
    } finally {
      setIsUpdatingMetaKey(false);
    }
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
            onSelect={(value) => {
              if (
                value !== "codex" &&
                value !== "gemini" &&
                value !== "meta" &&
                value !== "meta_app"
              )
                return;
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
          <div className="flex min-w-0 items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-end text-sm text-text/80">
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
                className="shrink-0"
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
                  className="shrink-0"
                  onClick={() => void openAntigravity()}
                >
                  {t("settings.transcription.openAntigravity")}
                </Button>
              )}
          </div>
        </SettingContainer>

        <SettingContainer
          title={t("settings.transcription.meta")}
          description={t("settings.transcription.metaDescription")}
          grouped={true}
        >
          <div className="flex w-full min-w-0 items-center gap-2">
            {metaStatus?.configured ? (
              <>
                <span className="min-w-0 flex-1 truncate text-end text-sm text-text/80">
                  {t("settings.transcription.configured")}
                </span>
                <Button
                  size="sm"
                  variant="danger-ghost"
                  disabled={provider === "meta" || isUpdatingMetaKey}
                  title={
                    provider === "meta"
                      ? t("settings.transcription.removeMetaApiKeyDisabled")
                      : undefined
                  }
                  onClick={() => void clearMetaApiKey()}
                >
                  {t("common.delete")}
                </Button>
              </>
            ) : (
              <>
                <Input
                  type="password"
                  value={metaApiKey}
                  onChange={(event) => setMetaApiKey(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void saveMetaApiKey();
                  }}
                  aria-label={t("settings.transcription.metaApiKeyPlaceholder")}
                  placeholder={t(
                    "settings.transcription.metaApiKeyPlaceholder",
                  )}
                  autoComplete="off"
                  spellCheck={false}
                  className="w-24 min-w-0 flex-1"
                  variant="compact"
                  disabled={isUpdatingMetaKey}
                />
                <Button
                  size="sm"
                  variant="secondary"
                  className="shrink-0"
                  disabled={!metaApiKey.trim() || isUpdatingMetaKey}
                  onClick={() => void saveMetaApiKey()}
                >
                  {t("settings.transcription.saveMetaApiKey")}
                </Button>
              </>
            )}
            <Button
              size="sm"
              variant="secondary"
              className="shrink-0"
              onClick={() => void openUrl("https://dev.meta.ai/")}
            >
              {t("common.open")}
            </Button>
          </div>
        </SettingContainer>

        <SettingContainer
          title={t("settings.transcription.metaApp")}
          description={t("settings.transcription.metaAppDescription")}
          grouped={true}
        >
          <div className="flex w-full min-w-0 items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-end text-sm text-text/80">
              {metaAppSessionLabel(metaAppStatus)}
            </span>
            {!metaAppStatus?.installed ? (
              <Button
                size="sm"
                variant="secondary"
                className="shrink-0"
                onClick={() => void openUrl("https://www.meta.ai/download/")}
              >
                {t("settings.transcription.installMetaApp")}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="secondary"
                className="shrink-0"
                onClick={() => void openMetaAi()}
              >
                {t("settings.transcription.openMetaApp")}
              </Button>
            )}
          </div>
        </SettingContainer>
      </SettingsGroup>

      {provider === "meta_app" ? (
        <SettingsGroup title={t("settings.transcription.groups.behavior")}>
          <SettingContainer
            title={t("settings.transcription.metaAppDirectTitle")}
            description={t("settings.transcription.metaAppDirectDescription")}
            grouped={true}
          >
            <span />
          </SettingContainer>
        </SettingsGroup>
      ) : (
        <>
          <SettingsGroup title={t("settings.transcription.groups.processing")}>
            <FillerWordRemoval descriptionMode="tooltip" grouped={true} />
            <CustomWords descriptionMode="tooltip" grouped={true} />
            <AppendTrailingSpace descriptionMode="tooltip" grouped={true} />
          </SettingsGroup>

          <SettingsGroup title={t("settings.transcription.groups.output")}>
            <PasteMethodSetting descriptionMode="tooltip" grouped={true} />
            <ClipboardHandlingSetting
              descriptionMode="tooltip"
              grouped={true}
            />
            <AutoSubmit descriptionMode="tooltip" grouped={true} />
          </SettingsGroup>
        </>
      )}
    </SettingsPage>
  );
};
