import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type TranscriptionProvider } from "@/bindings";
import {
  isConfiguredProvider,
  type ProviderStatus,
  useProviderStatuses,
} from "@/components/transcription/providerStatus";
import { Button } from "../ui/Button";
import MurmurTextLogo from "../icons/MurmurTextLogo";

interface ProviderOnboardingProps {
  configuredProvider: TranscriptionProvider;
  onComplete: (provider: TranscriptionProvider) => Promise<boolean>;
}

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

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

/** Explicit provider selection shown after macOS permission onboarding. */
const ProviderOnboarding: React.FC<ProviderOnboardingProps> = ({
  configuredProvider,
  onComplete,
}) => {
  const { t } = useTranslation();
  const { invalidatePendingRefreshes, refreshStatuses, statuses } =
    useProviderStatuses();
  const [selectedProvider, setSelectedProvider] =
    useState<TranscriptionProvider>(configuredProvider);
  const [isCompleting, setIsCompleting] = useState(false);

  useEffect(() => {
    void refreshStatuses();
    const handleFocus = () => void refreshStatuses();
    window.addEventListener("focus", handleFocus);
    return () => {
      invalidatePendingRefreshes();
      window.removeEventListener("focus", handleFocus);
    };
  }, [invalidatePendingRefreshes, refreshStatuses]);

  const openAntigravity = useCallback(async () => {
    try {
      const result = await commands.openAntigravity();
      if (result.status === "error") {
        throw new Error(result.error);
      }
    } catch (error) {
      toast.error(t("onboarding.provider.openFailed"), {
        description: errorMessage(error),
      });
    }
  }, [t]);

  const installAntigravity = useCallback(async () => {
    try {
      await openUrl("https://antigravity.google/");
    } catch (error) {
      toast.error(t("onboarding.provider.installFailed"), {
        description: errorMessage(error),
      });
    }
  }, [t]);

  const openCodexSetup = useCallback(async () => {
    try {
      await openUrl("https://openai.com/codex/");
    } catch (error) {
      toast.error(t("onboarding.provider.codexSetupFailed"), {
        description: errorMessage(error),
      });
    }
  }, [t]);

  const selectedStatus = statuses[selectedProvider];
  const canComplete = !isCompleting && isConfiguredProvider(selectedStatus);

  const complete = useCallback(async () => {
    if (!canComplete) return;

    setIsCompleting(true);
    try {
      const completed = await onComplete(selectedProvider);
      if (!completed) {
        await refreshStatuses();
      }
    } finally {
      setIsCompleting(false);
    }
  }, [canComplete, onComplete, refreshStatuses, selectedProvider]);

  const renderProvider = (
    provider: TranscriptionProvider,
    title: string,
    description: string,
  ) => {
    const status = statuses[provider];
    const isSelected = selectedProvider === provider;
    const isAvailable = isConfiguredProvider(status);
    const isGemini = provider === "gemini";

    return (
      <div
        key={provider}
        className={`rounded-lg border p-3 transition-colors ${
          isSelected
            ? "border-logo-primary bg-logo-primary/10"
            : "border-mid-gray/20 bg-white/5"
        }`}
      >
        <button
          type="button"
          role="radio"
          aria-checked={isSelected}
          aria-label={title}
          disabled={!isAvailable || isCompleting}
          onClick={() => setSelectedProvider(provider)}
          className="flex w-full items-start gap-3 text-start disabled:cursor-not-allowed disabled:opacity-60"
        >
          <span
            aria-hidden="true"
            className={`mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border ${
              isSelected ? "border-logo-primary" : "border-mid-gray/80"
            }`}
          >
            {isSelected && (
              <span className="size-2 rounded-full bg-logo-primary" />
            )}
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex flex-wrap items-center gap-2 font-medium text-text">
              {title}
              {isGemini && (
                <span className="rounded bg-logo-primary/20 px-1.5 py-0.5 text-xs font-medium text-logo-primary">
                  {t("onboarding.provider.experimental")}
                </span>
              )}
            </span>
            <span className="mt-1 block text-sm text-text/65">
              {description}
            </span>
            <span className="mt-2 block text-sm text-text/80">
              {t(statusTranslationKey(status))}
            </span>
          </span>
        </button>
        {status.kind === "unavailable" && (
          <div className="mt-3 flex flex-wrap gap-2 ps-7">
            {!isGemini ? (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void openCodexSetup()}
              >
                {t("onboarding.provider.setupCodex")}
              </Button>
            ) : status.reason === "notInstalled" ? (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void installAntigravity()}
              >
                {t("settings.transcription.installAntigravity")}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void openAntigravity()}
              >
                {t("settings.transcription.openAntigravity")}
              </Button>
            )}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="h-[100dvh] min-h-screen w-screen overflow-y-auto p-4 sm:p-6">
      <div className="mx-auto flex min-h-full w-full max-w-md flex-col items-center justify-center gap-3">
        <MurmurTextLogo width={180} />
        <div className="w-full max-w-md">
          <div className="mb-3 text-center">
            <h2 className="text-xl font-semibold text-text">
              {t("onboarding.provider.title")}
            </h2>
            <p className="mt-2 text-pretty text-text/70">
              {t("onboarding.provider.description")}
            </p>
          </div>
          <div role="radiogroup" className="flex flex-col gap-3">
            {renderProvider(
              "codex",
              t("onboarding.providerLabels.codex"),
              t("settings.transcription.sessionDescription"),
            )}
            {renderProvider(
              "gemini",
              t("onboarding.providerLabels.antigravity"),
              t("onboarding.provider.antigravityDescription"),
            )}
          </div>
          {!canComplete && selectedStatus.kind !== "checking" && (
            <p className="mt-3 text-sm text-text/70" aria-live="polite">
              {t("onboarding.provider.selectionRequired")}
            </p>
          )}
          <div className="mt-3 flex justify-end gap-2">
            <Button
              variant="secondary"
              onClick={() => void refreshStatuses()}
              disabled={isCompleting}
            >
              {t("onboarding.provider.retry")}
            </Button>
            <Button onClick={() => void complete()} disabled={!canComplete}>
              {t("onboarding.provider.continue")}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ProviderOnboarding;
