import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  statusTranslationKey,
  useCodexStatus,
} from "@/components/transcription/providerStatus";
import { Button } from "../ui/Button";
import MurmurTextLogo from "../icons/MurmurTextLogo";

interface ProviderOnboardingProps {
  onComplete: () => Promise<boolean>;
  /** Dictation runs on this Mac, so the account only adds AI cleanup and
   * more languages, and the step can be skipped. */
  optional: boolean;
}

const ProviderOnboarding: React.FC<ProviderOnboardingProps> = ({
  onComplete,
  optional,
}) => {
  const { t } = useTranslation();
  const { invalidatePendingRefreshes, refreshStatus, status } =
    useCodexStatus();
  const [isCompleting, setIsCompleting] = useState(false);

  useEffect(() => {
    void refreshStatus();
    const handleFocus = () => void refreshStatus();
    window.addEventListener("focus", handleFocus);
    return () => {
      invalidatePendingRefreshes();
      window.removeEventListener("focus", handleFocus);
    };
  }, [invalidatePendingRefreshes, refreshStatus]);

  const openCodexSetup = useCallback(async () => {
    try {
      await openUrl("https://openai.com/codex/");
    } catch (error) {
      console.warn("Failed to open Codex setup:", error);
      toast.error(t("onboarding.provider.codexSetupFailed"));
    }
  }, [t]);

  const canComplete = !isCompleting && status.kind === "configured";
  const canSkip = optional && !isCompleting && status.kind !== "configured";

  const finish = useCallback(async () => {
    setIsCompleting(true);
    try {
      const completed = await onComplete();
      if (!completed) {
        await refreshStatus();
      }
    } finally {
      setIsCompleting(false);
    }
  }, [onComplete, refreshStatus]);

  const complete = useCallback(async () => {
    if (canComplete) await finish();
  }, [canComplete, finish]);

  const skip = useCallback(async () => {
    if (canSkip) await finish();
  }, [canSkip, finish]);

  return (
    <div
      data-testid="provider-onboarding"
      className="h-[100dvh] min-h-screen w-screen overflow-y-auto p-3 sm:p-6"
    >
      <div className="mx-auto flex min-h-full w-full max-w-xl flex-col items-center justify-center gap-2">
        <MurmurTextLogo width={160} />
        <div className="w-full max-w-xl">
          <div className="mb-2 text-center">
            <h2 className="text-xl font-semibold text-text">
              {optional
                ? t("onboarding.provider.optionalTitle")
                : t("onboarding.provider.title")}
            </h2>
            <p className="mt-1 text-pretty text-sm text-text/70">
              {optional
                ? t("onboarding.provider.optionalDescription")
                : t("onboarding.provider.description")}
            </p>
          </div>
          <div className="rounded-lg border border-mid-gray/20 bg-white/5 p-2">
            <span className="block font-medium text-text">
              {t("onboarding.providerLabels.codex")}
            </span>
            <span className="mt-0.5 block text-sm text-text/65">
              {optional
                ? t("onboarding.provider.optionalSession")
                : t("settings.transcription.sessionDescription")}
            </span>
            <span
              className="mt-1 block text-sm text-text/80"
              aria-live="polite"
            >
              {t(statusTranslationKey(status))}
            </span>
            {status.kind === "unavailable" && (
              <div className="mt-2 flex flex-wrap gap-2">
                <Button onClick={() => void openCodexSetup()}>
                  {t("onboarding.provider.setupCodex")}
                </Button>
              </div>
            )}
          </div>
          <div className="mt-2 flex justify-end gap-2">
            {optional && status.kind !== "configured" && (
              <Button onClick={() => void skip()} disabled={!canSkip}>
                {t("onboarding.provider.skip")}
              </Button>
            )}
            <Button
              onClick={() => void refreshStatus()}
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
